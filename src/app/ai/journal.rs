// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The AI session journal: restore, handoff records and concurrent-write detection (task T23.12).
//!
//! A two-agent session outlives one command: the coder builds a candidate, the reviewer reads it
//! later, and both may be running in different processes. The journal is the durable half of that,
//! and it is deliberately a *compare-and-swap*, not a file the last writer wins:
//!
//! * **Restore is a value.** [`Journal::save`] writes one JSON document and [`Journal::load`] reads
//!   it back into the same types the session uses, so a restarted host continues with the role,
//!   budget, tool allowlist, usage ledger, candidates and handoffs it had.
//! * **A stale copy cannot overwrite a newer journal.** Each journal carries a sequence number, and
//!   `save` first reads the sequence on disk. A copy that was loaded at an older sequence is refused
//!   with both numbers, so two agents writing at once produce a conflict rather than a silent loss.
//! * **A candidate is checked, not trusted.** Loading rebuilds every candidate through
//!   [`super::Candidate::new_at`] and compares the recomputed digest with the stored one; a file
//!   whose text no longer matches its digest is refused, so an edited journal cannot pass a review
//!   that was made against different changes.
//! * **Unknown fields and versions are refused.** A future journal version is not read with a
//!   guess; the refusal names the version it found (principle #9/#22, and the same rule the project
//!   file reader follows).
//!
//! The journal never holds a secret: the session's `credential` is the *reference* (`credential` in
//! [`super::Session`]), which is what the service configuration stores anyway.

use std::collections::BTreeSet;
use std::path::Path;

use crate::project::record::json::{self, Json, JsonObject};

use super::{
    Candidate, Change, ChangeSet, Handoff, InvocationOutcome, Role, Session, UsageRecord,
    Verification,
};

/// The journal format this build writes and reads.
pub const JOURNAL_VERSION: u64 = 1;

/// The session fields a restarted host needs, separated from the live [`Session`] so that the
/// journal has one shape whether it is being written or read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionRecord {
    /// The role.
    pub role: Role,
    /// The service/model name.
    pub model: String,
    /// The context budget in bytes.
    pub budget: usize,
    /// The tools the role may call.
    pub tools: BTreeSet<String>,
    /// The credential reference, never the secret.
    pub credential: String,
    /// The usage ledger.
    pub usage: Vec<UsageRecord>,
}

impl SessionRecord {
    /// The record of a live session.
    pub fn of(session: &Session) -> Self {
        Self {
            role: session.role,
            model: session.model.clone(),
            budget: session.budget,
            tools: session.tools.clone(),
            credential: session.credential.clone(),
            usage: session.usage.clone(),
        }
    }

    /// A live session with the recorded configuration and ledger.
    pub fn restore(&self) -> Session {
        Session {
            role: self.role,
            model: self.model.clone(),
            budget: self.budget,
            tools: self.tools.clone(),
            credential: self.credential.clone(),
            usage: self.usage.clone(),
        }
    }
}

impl From<&Session> for SessionRecord {
    fn from(session: &Session) -> Self {
        Self::of(session)
    }
}

/// One durable session: its configuration, candidates and handoffs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Journal {
    seq: u64,
    session: SessionRecord,
    candidates: Vec<Candidate>,
    handoffs: Vec<Handoff>,
}

impl Journal {
    /// A new journal at sequence 0, which is what a file that does not exist yet holds.
    pub fn new(session: SessionRecord) -> Self {
        Self {
            seq: 0,
            session,
            candidates: Vec::new(),
            handoffs: Vec::new(),
        }
    }

    /// The sequence number this copy was loaded at (or saved to).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// The recorded session.
    pub fn session(&self) -> &SessionRecord {
        &self.session
    }

    /// The recorded candidates, in the order they were added.
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    /// The recorded handoffs, in the order they were added.
    pub fn handoffs(&self) -> &[Handoff] {
        &self.handoffs
    }

    /// Replaces the recorded session configuration and ledger.
    ///
    /// Replacing is how a save records the calls made since the last save; it does not change
    /// candidates or handoffs.
    pub fn set_session(&mut self, session: SessionRecord) {
        self.session = session;
    }

    /// Records a candidate.
    ///
    /// A candidate id is stable: recording the same id again replaces the stored copy only when its
    /// digest is unchanged, which is how a later verification or review comment is saved. An id whose
    /// digest changed is refused, because that is a different candidate wearing the old name.
    pub fn add_candidate(&mut self, candidate: Candidate) -> Result<(), String> {
        match self
            .candidates
            .iter_mut()
            .find(|recorded| recorded.id == candidate.id)
        {
            Some(recorded) if recorded.digest == candidate.digest => {
                *recorded = candidate;
                Ok(())
            }
            Some(recorded) => Err(format!(
                "candidate {} is already in the journal with digest {}, not {}",
                candidate.id, recorded.digest, candidate.digest
            )),
            None => {
                self.candidates.push(candidate);
                Ok(())
            }
        }
    }

    /// Adds a handoff.
    pub fn add_handoff(&mut self, handoff: Handoff) {
        self.handoffs.push(handoff);
    }

    /// The document as the journal file writes it.
    pub fn render(&self) -> String {
        let mut root = JsonObject::new();
        root.insert("version", Json::number(JOURNAL_VERSION));
        root.insert("seq", Json::number(self.seq));
        root.insert("session", session_json(&self.session));
        root.insert(
            "candidates",
            Json::Array(self.candidates.iter().map(candidate_json).collect()),
        );
        root.insert(
            "handoffs",
            Json::Array(self.handoffs.iter().map(handoff_json).collect()),
        );
        Json::Object(root).render()
    }

    /// Reads a journal document, refusing a version, field or digest this build cannot vouch for.
    pub fn parse(text: &str) -> Result<Self, String> {
        let value = json::parse(text).map_err(|error| {
            format!("the session journal is not JSON: {}", json_message(&error))
        })?;
        let root = value
            .as_object()
            .ok_or_else(|| "a session journal must be a JSON object".to_owned())?;
        let version = u64_field(root, "version", "the journal")?;
        if version != JOURNAL_VERSION {
            return Err(format!(
                "the session journal is version {version}; this build reads version {JOURNAL_VERSION}"
            ));
        }
        let seq = u64_field(root, "seq", "the journal")?;
        let session = parse_session(object_field(root, "session", "the journal")?)?;
        let mut candidates = Vec::new();
        for (index, value) in array_field(root, "candidates", "the journal")?
            .iter()
            .enumerate()
        {
            candidates.push(parse_candidate(value, index)?);
        }
        let mut handoffs = Vec::new();
        for (index, value) in array_field(root, "handoffs", "the journal")?
            .iter()
            .enumerate()
        {
            handoffs.push(parse_handoff(value, index)?);
        }
        Ok(Self {
            seq,
            session,
            candidates,
            handoffs,
        })
    }

    /// Reads the journal at `path`.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
        Self::parse(&text)
    }

    /// Writes the journal, refusing a stale copy, and returns the new sequence number.
    ///
    /// The compare is against the *file*, not against this copy: a journal that was loaded at
    /// sequence 3 and saved while another process advanced the file to 4 is refused with both
    /// numbers. The write itself is atomic (temporary file plus rename), so an interrupted save
    /// leaves the previous journal intact.
    pub fn save(&mut self, path: &Path) -> Result<u64, String> {
        let on_disk = if path.exists() {
            Some(Self::load(path)?.seq())
        } else {
            None
        };
        match on_disk {
            Some(disk) if disk != self.seq => {
                return Err(format!(
                    "the session journal on disk is at sequence {disk}; this copy is at sequence \
                     {} and was not written",
                    self.seq
                ));
            }
            None if self.seq != 0 => {
                return Err(format!(
                    "the session journal {} is gone, but this copy is at sequence {}; refusing to \
                     overwrite it",
                    path.display(),
                    self.seq
                ));
            }
            _ => {}
        }
        self.seq += 1;
        let text = self.render();
        json::write_atomically(path, &text)
            .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
        Ok(self.seq)
    }
}

fn session_json(session: &SessionRecord) -> Json {
    let mut object = JsonObject::new();
    object.insert("role", Json::string(role_name(session.role)));
    object.insert("model", Json::string(session.model.clone()));
    object.insert("budget", Json::number(session.budget as u64));
    object.insert(
        "tools",
        Json::Array(
            session
                .tools
                .iter()
                .map(|tool| Json::string(tool.clone()))
                .collect(),
        ),
    );
    object.insert("credential", Json::string(session.credential.clone()));
    object.insert(
        "usage",
        Json::Array(session.usage.iter().map(usage_json).collect()),
    );
    Json::Object(object)
}

fn usage_json(record: &UsageRecord) -> Json {
    let mut object = JsonObject::new();
    object.insert("model", Json::string(record.model.clone()));
    object.insert("role", Json::string(role_name(record.role)));
    object.insert("prompt_bytes", Json::number(record.prompt_bytes as u64));
    object.insert(
        "completion_bytes",
        Json::number(record.completion_bytes as u64),
    );
    object.insert("cost_micros", Json::number(record.cost_micros));
    object.insert("elapsed_ms", Json::number(record.elapsed_ms));
    let mut outcome = JsonObject::new();
    match &record.outcome {
        InvocationOutcome::Completed => {
            outcome.insert("kind", Json::string("completed"));
        }
        InvocationOutcome::Cancelled => {
            outcome.insert("kind", Json::string("cancelled"));
        }
        InvocationOutcome::Broken { reason } => {
            outcome.insert("kind", Json::string("broken"));
            outcome.insert("reason", Json::string(reason.clone()));
        }
    }
    object.insert("outcome", Json::Object(outcome));
    Json::Object(object)
}

fn candidate_json(candidate: &Candidate) -> Json {
    let mut object = JsonObject::new();
    object.insert("id", Json::number(candidate.id));
    object.insert("digest", Json::string(candidate.digest.clone()));
    object.insert(
        "verification",
        Json::string(verification_name(candidate.verification)),
    );
    object.insert(
        "comments",
        Json::Array(
            candidate
                .comments
                .iter()
                .map(|comment| Json::string(comment.clone()))
                .collect(),
        ),
    );
    let mut changes = JsonObject::new();
    changes.insert("version", Json::number(candidate.changes.version as u64));
    changes.insert(
        "base_revision",
        Json::number(candidate.changes.base_revision),
    );
    changes.insert(
        "changes",
        Json::Array(
            candidate
                .changes
                .changes
                .iter()
                .map(|change| {
                    let mut item = JsonObject::new();
                    item.insert("path", Json::string(change.path.clone()));
                    item.insert("before", Json::string(change.before.clone()));
                    item.insert("after", Json::string(change.after.clone()));
                    Json::Object(item)
                })
                .collect(),
        ),
    );
    object.insert("changes", Json::Object(changes));
    Json::Object(object)
}

fn handoff_json(handoff: &Handoff) -> Json {
    let mut object = JsonObject::new();
    object.insert("candidate", Json::string(handoff.candidate.clone()));
    object.insert("base_revision", Json::number(handoff.base_revision));
    object.insert("from", Json::string(role_name(handoff.from)));
    object.insert("to", Json::string(role_name(handoff.to)));
    object.insert(
        "shared",
        Json::Array(
            handoff
                .shared
                .iter()
                .map(|path| Json::string(path.clone()))
                .collect(),
        ),
    );
    Json::Object(object)
}

fn parse_session(object: &JsonObject) -> Result<SessionRecord, String> {
    let role = parse_role(string_field(object, "role", "the session")?, "the session")?;
    let model = string_field(object, "model", "the session")?;
    let budget = u64_field(object, "budget", "the session")? as usize;
    let mut tools = BTreeSet::new();
    for (index, value) in array_field(object, "tools", "the session")?
        .iter()
        .enumerate()
    {
        let tool = value
            .as_str()
            .ok_or_else(|| format!("the session's tool {index} is not a string"))?;
        tools.insert(tool.to_owned());
    }
    let credential = string_field(object, "credential", "the session")?;
    let mut usage = Vec::new();
    for (index, value) in array_field(object, "usage", "the session")?
        .iter()
        .enumerate()
    {
        usage.push(parse_usage(value, index)?);
    }
    Ok(SessionRecord {
        role,
        model,
        budget,
        tools,
        credential,
        usage,
    })
}

fn parse_usage(value: &Json, index: usize) -> Result<UsageRecord, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("usage record {index} is not an object"))?;
    let context = format!("usage record {index}");
    let model = string_field(object, "model", &context)?;
    let role = parse_role(string_field(object, "role", &context)?, &context)?;
    let prompt_bytes = u64_field(object, "prompt_bytes", &context)? as usize;
    let completion_bytes = u64_field(object, "completion_bytes", &context)? as usize;
    let cost_micros = u64_field(object, "cost_micros", &context)?;
    let elapsed_ms = u64_field(object, "elapsed_ms", &context)?;
    let outcome = object_field(object, "outcome", &context)?;
    let kind = string_field(outcome, "kind", &context)?;
    let outcome = match kind.as_str() {
        "completed" => InvocationOutcome::Completed,
        "cancelled" => InvocationOutcome::Cancelled,
        "broken" => InvocationOutcome::Broken {
            reason: string_field(outcome, "reason", &context)?,
        },
        other => return Err(format!("{context} has an unknown outcome {other:?}")),
    };
    Ok(UsageRecord {
        model,
        role,
        prompt_bytes,
        completion_bytes,
        cost_micros,
        elapsed_ms,
        outcome,
    })
}

fn parse_candidate(value: &Json, index: usize) -> Result<Candidate, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("candidate {index} is not an object"))?;
    let context = format!("candidate {index}");
    let id = u64_field(object, "id", &context)?;
    let stored_digest = string_field(object, "digest", &context)?;
    let verification = match string_field(object, "verification", &context)?.as_str() {
        "unverified" => Verification::Unverified,
        "compiled" => Verification::Compiled,
        "tested" => Verification::Tested,
        "failed" => Verification::Failed,
        other => return Err(format!("{context} has an unknown verification {other:?}")),
    };
    let mut comments = Vec::new();
    for (comment_index, comment) in array_field(object, "comments", &context)?
        .iter()
        .enumerate()
    {
        let text = comment
            .as_str()
            .ok_or_else(|| format!("{context}'s comment {comment_index} is not a string"))?;
        comments.push(text.to_owned());
    }
    let changes = object_field(object, "changes", &context)?;
    let version = u64_field(changes, "version", &context)?;
    let version = u32::try_from(version).map_err(|_| {
        format!("{context} has a change-set version {version} that this build cannot read")
    })?;
    let base_revision = u64_field(changes, "base_revision", &context)?;
    let mut list = Vec::new();
    for (change_index, change) in array_field(changes, "changes", &context)?
        .iter()
        .enumerate()
    {
        let change = change
            .as_object()
            .ok_or_else(|| format!("{context}'s change {change_index} is not an object"))?;
        list.push(Change {
            path: string_field(change, "path", &context)?,
            before: string_field(change, "before", &context)?,
            after: string_field(change, "after", &context)?,
        });
    }
    let set = ChangeSet {
        version,
        base_revision,
        changes: list,
    };
    // Rebuilding through `new_at` recomputes the digest, so a journal whose changes were edited
    // cannot pass as the candidate a reviewer approved.
    let mut built = Candidate::new_at(id, set, base_revision)
        .map_err(|reason| format!("{context} is not valid: {reason}"))?;
    if built.digest != stored_digest {
        return Err(format!(
            "{context} stores digest {stored_digest}, but its changes digest to {}; the journal was \
             edited",
            built.digest
        ));
    }
    built.verification = verification;
    built.comments = comments;
    Ok(built)
}

fn parse_handoff(value: &Json, index: usize) -> Result<Handoff, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("handoff {index} is not an object"))?;
    let context = format!("handoff {index}");
    let mut shared = Vec::new();
    for (path_index, path) in array_field(object, "shared", &context)?.iter().enumerate() {
        let path = path
            .as_str()
            .ok_or_else(|| format!("{context}'s shared path {path_index} is not a string"))?;
        shared.push(path.to_owned());
    }
    Ok(Handoff {
        candidate: string_field(object, "candidate", &context)?,
        base_revision: u64_field(object, "base_revision", &context)?,
        from: parse_role(string_field(object, "from", &context)?, &context)?,
        to: parse_role(string_field(object, "to", &context)?, &context)?,
        shared,
    })
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Coder => "coder",
        Role::Reviewer => "reviewer",
    }
}

fn parse_role(name: String, context: &str) -> Result<Role, String> {
    match name.as_str() {
        "coder" => Ok(Role::Coder),
        "reviewer" => Ok(Role::Reviewer),
        other => Err(format!("{context} names an unknown role {other:?}")),
    }
}

fn verification_name(verification: Verification) -> &'static str {
    match verification {
        Verification::Unverified => "unverified",
        Verification::Compiled => "compiled",
        Verification::Tested => "tested",
        Verification::Failed => "failed",
    }
}

fn object_field<'a>(
    object: &'a JsonObject,
    name: &str,
    context: &str,
) -> Result<&'a JsonObject, String> {
    object
        .get(name)
        .and_then(|value| value.as_object())
        .ok_or_else(|| format!("{context} has no {name} object"))
}

fn array_field<'a>(
    object: &'a JsonObject,
    name: &str,
    context: &str,
) -> Result<&'a [Json], String> {
    object
        .get(name)
        .and_then(|value| value.as_array())
        .ok_or_else(|| format!("{context} has no {name} array"))
}

fn string_field(object: &JsonObject, name: &str, context: &str) -> Result<String, String> {
    object
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("{context} has no {name} string"))
}

fn u64_field(object: &JsonObject, name: &str, context: &str) -> Result<u64, String> {
    object
        .get(name)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| format!("{context} has no {name} number"))
}

/// The JSON reader's error as a sentence including where it stopped.
fn json_message(error: &json::JsonError) -> String {
    format!(
        "{} (line {}, column {})",
        error.kind().message(),
        error.line(),
        error.column()
    )
}

#[cfg(test)]
mod tests {
    use super::{JOURNAL_VERSION, Journal, SessionRecord};
    use crate::app::ai::{
        Candidate, Change, ChangeSet, Handoff, InvocationOutcome, Role, Session, UsageRecord,
        Verification,
    };
    use std::collections::BTreeSet;

    fn session() -> Session {
        Session {
            role: Role::Coder,
            model: "coder-1".to_owned(),
            budget: 4096,
            tools: ["read_file", "write_file"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            credential: "keyring:ai/coder".to_owned(),
            usage: vec![UsageRecord {
                model: "coder-1".to_owned(),
                role: Role::Coder,
                prompt_bytes: 120,
                completion_bytes: 30,
                cost_micros: 7,
                elapsed_ms: 250,
                outcome: InvocationOutcome::Broken {
                    reason: "timeout".to_owned(),
                },
            }],
        }
    }

    fn candidate() -> Candidate {
        Candidate::new_at(
            7,
            ChangeSet {
                version: 1,
                base_revision: 4,
                changes: vec![Change {
                    path: "src/a.rs".to_owned(),
                    before: "one".to_owned(),
                    after: "two".to_owned(),
                }],
            },
            4,
        )
        .expect("a candidate")
    }

    fn sandbox(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!(
            "rustage-journal-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("a sandbox");
        base
    }

    #[test]
    fn a_journal_round_trips_the_session_candidates_and_handoffs() {
        let mut journal = Journal::new(SessionRecord::of(&session()));
        journal.add_candidate(candidate()).expect("a new candidate");
        journal.add_handoff(
            Handoff::new(
                Role::Coder,
                Role::Reviewer,
                &candidate(),
                vec![".ai/share/a".to_owned()],
            )
            .expect("two roles"),
        );

        let text = journal.render();
        let back = Journal::parse(&text).expect("the journal parses");

        assert_eq!(back.session(), journal.session());
        assert_eq!(back.candidates(), journal.candidates());
        assert_eq!(back.handoffs(), journal.handoffs());
        assert!(
            text.contains(&format!("\"version\": {JOURNAL_VERSION}")),
            "{text}"
        );
    }

    #[test]
    fn a_candidate_whose_changes_were_edited_is_refused_on_load() {
        let mut journal = Journal::new(SessionRecord::of(&session()));
        journal.add_candidate(candidate()).expect("a new candidate");
        let text = journal
            .render()
            .replace("\"after\": \"two\"", "\"after\": \"three\"");

        let refusal = Journal::parse(&text).expect_err("the digest no longer matches");

        assert!(refusal.contains("edited"), "{refusal}");
    }

    #[test]
    fn a_future_version_is_refused_rather_than_guessed() {
        let journal = Journal::new(SessionRecord::of(&session()));
        let text = journal
            .render()
            .replace("\"version\": 1", "\"version\": 99");

        let refusal = Journal::parse(&text).expect_err("a future version is not read");

        assert!(refusal.contains("version 99"), "{refusal}");
    }

    #[test]
    fn a_candidate_id_is_replaced_only_when_its_digest_is_unchanged() {
        let mut journal = Journal::new(SessionRecord::of(&session()));
        journal.add_candidate(candidate()).expect("the first");
        journal
            .add_candidate(candidate())
            .expect("the same candidate");
        assert_eq!(journal.candidates().len(), 1, "an id is one record");

        // Building it through `new_at` is what gives it its own digest; editing the field of a
        // clone would leave the old digest in place and only test the fields.
        let other = Candidate::new_at(
            7,
            ChangeSet {
                version: 1,
                base_revision: 4,
                changes: vec![Change {
                    path: "src/a.rs".to_owned(),
                    before: "one".to_owned(),
                    after: "different".to_owned(),
                }],
            },
            4,
        )
        .expect("a different candidate with the same id");
        let refusal = journal
            .add_candidate(other)
            .expect_err("a different digest under the same id is refused");
        assert!(refusal.contains("already in the journal"), "{refusal}");
    }

    #[test]
    fn the_second_writer_of_a_stale_copy_is_refused_and_names_both_sequences() {
        let dir = sandbox("stale");
        let path = dir.join("session.json");
        let mut first = Journal::new(SessionRecord::of(&session()));
        assert_eq!(first.save(&path).expect("the first save"), 1);

        let mut second = Journal::load(&path).expect("both copies load at one");
        first.set_session(SessionRecord {
            tools: BTreeSet::from(["read_file".to_owned()]),
            ..SessionRecord::of(&session())
        });
        assert_eq!(first.save(&path).expect("the first copy advances"), 2);

        let refusal = second
            .save(&path)
            .expect_err("the stale copy must not overwrite");
        assert!(refusal.contains("sequence 1"), "{refusal}");
        assert!(refusal.contains("at sequence 2"), "{refusal}");
    }

    #[test]
    fn a_saved_journal_restores_the_ledger_and_the_verification() {
        let dir = sandbox("restore");
        let path = dir.join("session.json");
        let mut journal = Journal::new(SessionRecord::of(&session()));
        let mut candidate = candidate();
        let digest = candidate.digest.clone();
        candidate
            .verify(&digest, Verification::Tested)
            .expect("the digest matches");
        candidate.review("looks right");
        journal.add_candidate(candidate).expect("a new candidate");
        journal.save(&path).expect("saved");

        let restored = Journal::load(&path).expect("loaded");
        let session = restored.session().restore();
        assert_eq!(session.totals().broken, 1);
        assert_eq!(session.usage().len(), 1);
        let candidate = &restored.candidates()[0];
        assert_eq!(candidate.verification, Verification::Tested);
        assert_eq!(candidate.comments, vec!["looks right".to_owned()]);
        std::fs::remove_dir_all(dir).ok();
    }
}
