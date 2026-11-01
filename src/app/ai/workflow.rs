// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The optional AI seam: context authorization, structured candidates and human review (task F23).
//!
//! The designer works without any of this module; when an agent is enabled, the host — not the
//! model — decides what is sent, which candidate is current and what is written. A candidate is
//! immutable once created and carries the revision it was based on and a digest of its changes, so
//! an old verification cannot be reused for a new candidate. Accepting is by explicit change index,
//! and every accepted change keeps its previous text for rollback.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

/// Which agent role a workspace belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Writes the candidate.
    Coder,
    /// Reviews a candidate independently.
    Reviewer,
}

impl Role {
    /// The role's directory under `.ai`.
    pub fn directory(self) -> &'static str {
        match self {
            Self::Coder => ".ai/coder",
            Self::Reviewer => ".ai/reviewer",
        }
    }

    /// The shared material directory.
    pub fn shared() -> &'static str {
        ".ai/share"
    }
}

/// One file offered to the model.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ContextFile {
    /// The project-relative path.
    pub path: String,
    /// Its text, before any redaction.
    pub text: String,
}

/// A context selection after the host's filters.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Context {
    /// The files that may be sent.
    pub files: Vec<ContextFile>,
    /// The paths the host ignored.
    pub ignored: Vec<String>,
    /// The paths whose text had secret-looking lines removed.
    pub redacted: Vec<String>,
    /// Whether a person authorized this exact selection.
    pub authorized: bool,
}

impl Context {
    /// Selects `files`, ignoring `patterns` and removing secret-looking lines.
    ///
    /// A line is secret-looking when its key (before `=` or `:`) contains one of `secret`, `token`,
    /// `password`, `passwd` or `api_key`. The line is replaced with `[redacted]`, so the model sees
    /// that something was there rather than a silently shorter file.
    pub fn select(files: Vec<ContextFile>, ignored: &[String]) -> Self {
        let mut selected = Vec::new();
        let mut ignored_paths = Vec::new();
        let mut redacted = Vec::new();
        for file in files {
            if ignored
                .iter()
                .any(|pattern| file.path.contains(pattern.as_str()))
            {
                ignored_paths.push(file.path);
                continue;
            }
            let (text, changed) = redact(&file.text);
            if changed {
                redacted.push(file.path.clone());
            }
            selected.push(ContextFile {
                path: file.path,
                text,
            });
        }
        Self {
            files: selected,
            ignored: ignored_paths,
            redacted,
            authorized: false,
        }
    }

    /// Marks the selection as explicitly authorized by a person.
    pub fn authorize(&mut self) {
        self.authorized = true;
    }

    /// The bytes that would be sent.
    fn payload(&self) -> String {
        self.files
            .iter()
            .map(|file| format!("--- {}\n{}", file.path, file.text))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The payload size, without requiring the caller to authorize an export.
    pub fn payload_len(&self) -> usize {
        self.payload().len()
    }

    /// The bytes a model call may send.
    ///
    /// This is the only public way to obtain the payload: a selection that was not explicitly
    /// authorized by a person refuses with [`SendError::Unauthorized`], so a caller cannot send
    /// material merely by holding a [`Context`].
    pub fn sendable_payload(&self) -> Result<String, SendError> {
        if !self.authorized {
            return Err(SendError::Unauthorized);
        }
        Ok(self.payload())
    }
}

/// Why a context cannot be sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SendError {
    /// No person authorized this selection.
    Unauthorized,
}

impl core::fmt::Display for SendError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized => formatter.write_str("the context was not authorized"),
        }
    }
}

fn redact(text: &str) -> (String, bool) {
    let mut changed = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let key = line
            .split_once('=')
            .or_else(|| line.split_once(':'))
            .map(|(key, _)| key.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let secret = ["secret", "token", "password", "passwd", "api_key"]
            .iter()
            .any(|word| key.contains(word));
        if secret {
            out.push("[redacted]".to_owned());
            changed = true;
        } else {
            out.push(line.to_owned());
        }
    }
    (out.join("\n"), changed)
}

/// One change in a candidate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// The project-relative path.
    pub path: String,
    /// The text before the change.
    pub before: String,
    /// The text after it.
    pub after: String,
}

/// A versioned set of changes based on one document revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangeSet {
    /// The change-set format version.
    pub version: u32,
    /// The revision it was based on.
    pub base_revision: u64,
    /// The changes in order.
    pub changes: Vec<Change>,
}

impl ChangeSet {
    /// Validates paths, duplicates, the base revision and the format version.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!(
                "change-set version {} is not supported",
                self.version
            ));
        }
        if self.base_revision == 0 {
            return Err("a change set needs a non-zero base revision".to_owned());
        }
        if self.changes.is_empty() {
            return Err("a change set needs at least one change".to_owned());
        }
        let mut seen = BTreeSet::new();
        for change in &self.changes {
            if change.path.trim().is_empty() {
                return Err("a change has an empty path".to_owned());
            }
            if change.path.starts_with('/') || change.path.split('/').any(|part| part == "..") {
                return Err(format!("{} escapes the project root", change.path));
            }
            if is_generated_zone(&change.path) {
                return Err(format!(
                    "{} is in the generated zone; an agent may not write it",
                    change.path
                ));
            }
            if !seen.insert(change.path.clone()) {
                return Err(format!("{} appears twice in the change set", change.path));
            }
        }
        Ok(())
    }

    /// Validates the set against the model revision it claims to be based on.
    ///
    /// A candidate produced from an older model must not be applied to a newer one, even when its
    /// paths are valid: the `before` text guard in [`Workspace::apply`] catches a changed file, but
    /// the revision check catches the same mistake before any change is offered for acceptance.
    pub fn validate_against(&self, current_revision: u64) -> Result<(), String> {
        self.validate()?;
        if self.base_revision != current_revision {
            return Err(format!(
                "the change set is based on revision {}, but the document is at {}",
                self.base_revision, current_revision
            ));
        }
        Ok(())
    }

    /// Validates a set against a real project root, including symbolic links.
    ///
    /// [`Self::validate`] refuses a lexical `..` escape, but a path inside the root can still leave
    /// it through a symlink. Every existing path component is checked: a symlink whose resolved
    /// target is outside the root is refused and named. A component that does not exist yet is not
    /// an error, because an agent may create a new file; the existing prefix has already been
    /// checked.
    pub fn validate_in(&self, root: &std::path::Path) -> Result<(), String> {
        self.validate()?;
        let root = root.canonicalize().map_err(|error| {
            format!(
                "the project root {} cannot be resolved: {error}",
                root.display()
            )
        })?;
        for change in &self.changes {
            let mut candidate = root.clone();
            for part in change.path.split('/').filter(|part| !part.is_empty()) {
                candidate.push(part);
                let Ok(metadata) = std::fs::symlink_metadata(&candidate) else {
                    break;
                };
                if !metadata.file_type().is_symlink() {
                    continue;
                }
                let resolved = candidate.canonicalize().map_err(|error| {
                    format!(
                        "{} is a symbolic link that cannot be resolved: {error}",
                        change.path
                    )
                })?;
                if !resolved.starts_with(&root) {
                    return Err(format!(
                        "{} is a symbolic link that leaves the project root (resolves to {})",
                        change.path,
                        resolved.display()
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The directories the generator owns; an agent change may not name them.
fn is_generated_zone(path: &str) -> bool {
    let path = path.strip_prefix("./").unwrap_or(path);
    path == "src/generated"
        || path.starts_with("src/generated/")
        || path == "target"
        || path.starts_with("target/")
}

/// Why a candidate cannot be accepted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verification {
    /// Not verified yet.
    Unverified,
    /// The compile step passed.
    Compiled,
    /// Compile and tests passed.
    Tested,
    /// It failed and may not be accepted.
    Failed,
}

/// A candidate is immutable after construction.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidate {
    /// The candidate id.
    pub id: u64,
    /// The changes it proposes.
    pub changes: ChangeSet,
    /// A digest of the changes, fixed at construction.
    pub digest: String,
    /// What verification reached.
    pub verification: Verification,
    /// The review comments.
    pub comments: Vec<String>,
}

impl Candidate {
    /// Creates a candidate only when its base matches the current model revision.
    pub fn new_at(id: u64, changes: ChangeSet, current_revision: u64) -> Result<Self, String> {
        changes.validate_against(current_revision)?;
        Self::seal(id, changes)
    }

    fn seal(id: u64, changes: ChangeSet) -> Result<Self, String> {
        let mut hasher = Sha256::new();
        hasher.update(changes.version.to_le_bytes());
        hasher.update(changes.base_revision.to_le_bytes());
        for change in &changes.changes {
            hasher.update((change.path.len() as u64).to_le_bytes());
            hasher.update(change.path.as_bytes());
            hasher.update((change.before.len() as u64).to_le_bytes());
            hasher.update(change.before.as_bytes());
            hasher.update((change.after.len() as u64).to_le_bytes());
            hasher.update(change.after.as_bytes());
        }
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            id,
            changes,
            digest,
            verification: Verification::Unverified,
            comments: Vec::new(),
        })
    }

    /// Records a verification result only if the candidate's digest is unchanged.
    pub fn verify(&mut self, digest: &str, result: Verification) -> Result<(), String> {
        if digest != self.digest {
            return Err(format!(
                "the verification is for {digest}, not for {}",
                self.digest
            ));
        }
        if result == Verification::Failed {
            self.verification = Verification::Failed;
        } else {
            self.verification = result;
        }
        Ok(())
    }

    /// Adds a review comment.
    pub fn review(&mut self, comment: impl Into<String>) {
        self.comments.push(comment.into());
    }

    /// Accepts only the changes at `indices`, returning the accepted changes and the rollback text.
    ///
    /// An index that does not exist is refused before anything is accepted. A `Failed` candidate
    /// cannot be accepted at all.
    pub fn accept(&self, indices: &[usize]) -> Result<Vec<Change>, String> {
        if self.verification == Verification::Failed {
            return Err("a failed candidate cannot be accepted".to_owned());
        }
        if indices.is_empty() {
            return Err("accepting needs at least one change index".to_owned());
        }
        for index in indices {
            if *index >= self.changes.changes.len() {
                return Err(format!(
                    "change index {index} is outside {} change(s)",
                    self.changes.changes.len()
                ));
            }
        }
        Ok(indices
            .iter()
            .map(|index| self.changes.changes[*index].clone())
            .collect())
    }
}

/// The current documents after an acceptance, with the text needed to undo it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Workspace {
    files: BTreeMap<String, String>,
}

impl Workspace {
    /// A workspace with the given files.
    pub fn new(files: BTreeMap<String, String>) -> Self {
        Self { files }
    }

    /// Applies accepted changes and returns the previous text for each path.
    pub fn apply(&mut self, changes: &[Change]) -> Result<BTreeMap<String, String>, String> {
        for change in changes {
            let current = self.files.get(&change.path);
            if let Some(current) = current
                && current != &change.before
            {
                return Err(format!("{} changed underneath the candidate", change.path));
            }
        }
        let mut undo = BTreeMap::new();
        for change in changes {
            let previous = self
                .files
                .insert(change.path.clone(), change.after.clone())
                .unwrap_or_default();
            undo.insert(change.path.clone(), previous);
        }
        Ok(undo)
    }

    /// Puts the previous text back.
    pub fn rollback(&mut self, undo: &BTreeMap<String, String>) {
        for (path, text) in undo {
            self.files.insert(path.clone(), text.clone());
        }
    }

    /// The text of one file.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }
}

/// A role's service configuration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    /// The role.
    pub role: Role,
    /// The service/model name.
    pub model: String,
    /// The context budget in bytes.
    pub budget: usize,
    /// The tools the role may call.
    pub tools: BTreeSet<String>,
    /// The credential reference (never the secret itself).
    pub credential: String,
    /// What this session's calls consumed and how they ended (T23.07).
    pub usage: Vec<UsageRecord>,
}

impl Session {
    /// Whether the context fits the budget.
    pub fn fits(&self, context: &Context) -> bool {
        context.payload_len() <= self.budget
    }

    /// Whether a tool is allowed.
    pub fn allows(&self, tool: &str) -> bool {
        self.tools.contains(tool)
    }

    /// Builds the call preview for a tool this session may use.
    ///
    /// A tool outside the session's allowlist is refused here, before a person is asked to
    /// authorize anything — the allowlist and the per-call grant are two separate gates.
    pub fn preview_tool(&self, name: &str, arguments: &str) -> Result<ToolCall, String> {
        if !self.allows(name) {
            return Err(format!("tool {name:?} is not in this session's tool set"));
        }
        Ok(ToolCall {
            role: self.role,
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        })
    }

    /// Records one call.
    ///
    /// The record must name this session's model: a ledger that mixes models cannot answer "what
    /// did this service cost", so a mismatched record is refused rather than silently filed.
    pub fn record(&mut self, record: UsageRecord) -> Result<(), String> {
        if record.model != self.model {
            return Err(format!(
                "the record names model {:?}, not this session's {:?}",
                record.model, self.model
            ));
        }
        self.usage.push(record);
        Ok(())
    }

    /// The recorded calls, in order.
    pub fn usage(&self) -> &[UsageRecord] {
        &self.usage
    }

    /// The ledger totals.
    pub fn totals(&self) -> UsageTotals {
        let mut totals = UsageTotals::default();
        for record in &self.usage {
            totals.prompt_bytes += record.prompt_bytes;
            totals.completion_bytes += record.completion_bytes;
            totals.cost_micros += record.cost_micros;
            totals.elapsed_ms += record.elapsed_ms;
            match record.outcome {
                InvocationOutcome::Completed => totals.completed += 1,
                InvocationOutcome::Cancelled => totals.cancelled += 1,
                InvocationOutcome::Broken { .. } => totals.broken += 1,
            }
        }
        totals
    }
}

/// A coder→reviewer handoff (T23.12).
///
/// The handoff names the candidate by digest and the revision it was based on. A reviewer's copy
/// therefore cannot silently review a different candidate, and a document that moved on since the
/// handoff is a conflict rather than an approval. The shared material this transfer used is listed
/// so the host can clear it when the review ends; the model only says which paths it is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Handoff {
    /// The candidate digest the reviewer is asked to review.
    pub candidate: String,
    /// The document revision the candidate was based on.
    pub base_revision: u64,
    /// The role handing the candidate over.
    pub from: Role,
    /// The role receiving it.
    pub to: Role,
    /// Shared-material paths used for this transfer.
    pub shared: Vec<String>,
}

impl Handoff {
    /// Builds a handoff between two different roles.
    pub fn new(
        from: Role,
        to: Role,
        candidate: &Candidate,
        shared: Vec<String>,
    ) -> Result<Self, String> {
        if from == to {
            return Err("a handoff needs two different roles".to_owned());
        }
        Ok(Self {
            candidate: candidate.digest.clone(),
            base_revision: candidate.changes.base_revision,
            from,
            to,
            shared,
        })
    }

    /// Checks that the reviewer still holds the handed-over candidate and revision.
    pub fn check(&self, candidate: &Candidate, current_revision: u64) -> Result<(), String> {
        if self.candidate != candidate.digest {
            return Err(format!(
                "the handoff is for {}, but the candidate is {}",
                self.candidate, candidate.digest
            ));
        }
        if self.base_revision != current_revision {
            return Err(format!(
                "the handoff is based on revision {}, but the document is at {}",
                self.base_revision, current_revision
            ));
        }
        Ok(())
    }

    /// The shared-material paths the host may clear once the review is over.
    pub fn cleanup(&self) -> &[String] {
        &self.shared
    }
}

/// How one model call ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InvocationOutcome {
    /// The service answered in full.
    Completed,
    /// A person cancelled it.
    Cancelled,
    /// The stream ended before the answer was complete, with the reason the service gave.
    Broken {
        /// Why the stream stopped.
        reason: String,
    },
}

impl InvocationOutcome {
    /// The outcome's name for a report.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Broken { .. } => "broken",
        }
    }
}

/// One model call's consumption and outcome.
///
/// `cost_micros` is what the service reported, not a price table guessed by the host: a missing
/// price is zero with the model named in the record, not a fabricated estimate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UsageRecord {
    /// The model the call addressed.
    pub model: String,
    /// The role that made it.
    pub role: Role,
    /// Bytes of prompt context sent.
    pub prompt_bytes: usize,
    /// Bytes of completion received.
    pub completion_bytes: usize,
    /// Cost in millionths of the billing unit, as reported by the service; 0 when it did not.
    pub cost_micros: u64,
    /// Wall-clock duration of the call.
    pub elapsed_ms: u64,
    /// How it ended.
    pub outcome: InvocationOutcome,
}

impl UsageRecord {
    /// The record in one line.
    pub fn render(&self) -> String {
        let reason = match &self.outcome {
            InvocationOutcome::Broken { reason } => format!(" ({reason})"),
            _ => String::new(),
        };
        format!(
            "{} {}: {} prompt / {} completion byte(s), {} us, {} ms — {}{reason}",
            self.role.directory(),
            self.model,
            self.prompt_bytes,
            self.completion_bytes,
            self.cost_micros,
            self.elapsed_ms,
            self.outcome.name(),
        )
    }
}

/// The session ledger's totals.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct UsageTotals {
    /// Prompt bytes across every call.
    pub prompt_bytes: usize,
    /// Completion bytes across every call.
    pub completion_bytes: usize,
    /// Reported cost across every call.
    pub cost_micros: u64,
    /// Wall-clock milliseconds across every call.
    pub elapsed_ms: u64,
    /// Calls that completed.
    pub completed: usize,
    /// Calls a person cancelled.
    pub cancelled: usize,
    /// Calls whose stream broke.
    pub broken: usize,
}

impl UsageTotals {
    /// The totals in one line.
    pub fn render(&self) -> String {
        format!(
            "{} call(s): {} completed, {} cancelled, {} broken | {} prompt / {} completion byte(s) | {} us | {} ms",
            self.completed + self.cancelled + self.broken,
            self.completed,
            self.cancelled,
            self.broken,
            self.prompt_bytes,
            self.completion_bytes,
            self.cost_micros,
            self.elapsed_ms
        )
    }
}

/// One external tool call a session proposes (T23.11).
///
/// The call is data: constructing it does not run anything. The host previews it, authorizes that
/// exact name-and-arguments pair, and only then may a transport send it. There is no ambient
/// “the tool is allowed” permission — a grant for `read_file` with one path does not authorize
/// another path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ToolCall {
    /// The role that wants to call it.
    pub role: Role,
    /// The tool name the transport will use.
    pub name: String,
    /// The arguments, as the JSON text the tool expects.
    pub arguments: String,
}

impl ToolCall {
    /// A digest of the exact call, used as the authorization key.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update((self.name.len() as u64).to_le_bytes());
        hasher.update(self.name.as_bytes());
        hasher.update((self.arguments.len() as u64).to_le_bytes());
        hasher.update(self.arguments.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// The human preview a person authorizes against.
    pub fn preview(&self) -> String {
        format!(
            "{} wants to call {} with {}: {}",
            self.role.directory(),
            self.name,
            redact_arguments(&self.arguments),
            self.digest()
        )
    }

    /// The MCP `tools/call` request body for this call.
    ///
    /// The JSON-RPC `id` belongs to the transport (it pairs the answer with the request), so this
    /// seam does not invent one; a transport that needs one adds it around this body.
    ///
    /// MCP defines `arguments` as a map, so the call's text has to be a JSON object. Text that is not
    /// one is refused rather than embedded as a string that no server would read as arguments.
    pub fn mcp_json(&self) -> Result<String, String> {
        let params = self.mcp_params()?;
        let mut root = crate::project::record::json::JsonObject::new();
        root.insert(
            "method",
            crate::project::record::json::Json::string("tools/call"),
        );
        root.insert("params", crate::project::record::json::Json::Object(params));
        Ok(crate::project::record::json::Json::Object(root).render_compact())
    }

    /// The `params` member of the MCP request.
    ///
    /// Shared by the preview body and the stdio transport, so the bytes a person reviews and the
    /// bytes a server receives cannot disagree (principle #59).
    pub(super) fn mcp_params(&self) -> Result<crate::project::record::json::JsonObject, String> {
        use crate::project::record::json::{self, Json, JsonObject};
        let arguments = json::parse(&self.arguments).map_err(|error| {
            format!(
                "the arguments of {} are not JSON: {} (line {}, column {})",
                self.name,
                error.kind().message(),
                error.line(),
                error.column()
            )
        })?;
        if !matches!(arguments, Json::Object(_)) {
            return Err(format!(
                "the arguments of {} are not a JSON object, which MCP requires",
                self.name
            ));
        }
        let mut params = JsonObject::new();
        params.insert("name", Json::string(self.name.clone()));
        params.insert("arguments", arguments);
        Ok(params)
    }
}

/// Redacts values of secret-looking argument keys in a preview.
fn redact_arguments(arguments: &str) -> String {
    let mut out = String::with_capacity(arguments.len());
    for part in arguments.split(',') {
        let key = part
            .split_once('=')
            .or_else(|| part.split_once(':'))
            .map(|(key, _)| key.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let secret = ["secret", "token", "password", "passwd", "api_key"]
            .iter()
            .any(|word| key.contains(word));
        if secret {
            out.push_str("[redacted]");
        } else {
            out.push_str(part);
        }
        out.push(',');
    }
    out.pop();
    out
}

/// Whether a proposed call may be sent.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ToolDecision {
    /// A person authorized this exact call.
    Allowed,
    /// No grant covers this exact call.
    Refused {
        /// Why.
        reason: String,
    },
}

impl ToolDecision {
    /// Whether the call may be sent.
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// Per-call grants for external tools.
///
/// A grant names one digest (tool + exact arguments). Authorizing one call therefore cannot be
/// reused for another path, value or tool.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ToolPolicy {
    granted: BTreeSet<String>,
}

impl ToolPolicy {
    /// Authorizes one exact call, as a person's explicit action.
    pub fn authorize(&mut self, call: &ToolCall) {
        self.granted.insert(call.digest());
    }

    /// Withdraws every grant.
    pub fn clear(&mut self) {
        self.granted.clear();
    }

    /// Decides whether a call may be sent.
    pub fn decide(&self, call: &ToolCall) -> ToolDecision {
        if self.granted.contains(&call.digest()) {
            ToolDecision::Allowed
        } else {
            ToolDecision::Refused {
                reason: format!(
                    "no person authorized {} with these exact arguments",
                    call.name
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Candidate, Change, ChangeSet, Context, ContextFile, Handoff, InvocationOutcome, Role,
        Session, ToolPolicy, UsageRecord, Verification, Workspace,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn change(path: &str, before: &str, after: &str) -> Change {
        Change {
            path: path.to_owned(),
            before: before.to_owned(),
            after: after.to_owned(),
        }
    }

    fn candidate() -> Candidate {
        Candidate::new_at(
            1,
            ChangeSet {
                version: 1,
                base_revision: 4,
                changes: vec![change("a.rs", "one", "two"), change("b.rs", "", "new")],
            },
            4,
        )
        .expect("a candidate")
    }

    #[test]
    fn context_ignores_paths_and_redacts_secret_lines() {
        let context = Context::select(
            vec![
                ContextFile {
                    path: "src/a.rs".to_owned(),
                    text: "let x = 1;\nlet token = \"secret\";".to_owned(),
                },
                ContextFile {
                    path: "target/build.log".to_owned(),
                    text: "noise".to_owned(),
                },
            ],
            &["target".to_owned()],
        );
        assert_eq!(context.files.len(), 1);
        assert!(context.files[0].text.contains("[redacted]"));
        assert!(context.redacted.contains(&"src/a.rs".to_owned()));
        assert!(context.ignored.contains(&"target/build.log".to_owned()));
    }

    #[test]
    fn a_change_set_refuses_bad_paths_duplicates_and_an_old_version() {
        let mut set = ChangeSet {
            version: 2,
            base_revision: 1,
            changes: vec![change("a.rs", "", "x")],
        };
        assert!(set.validate().is_err());
        set.version = 1;
        set.changes.push(change("../escape.rs", "", "x"));
        assert!(set.validate().is_err());
        set.changes = vec![change("a.rs", "", "x"), change("a.rs", "", "y")];
        assert!(set.validate().is_err());
    }

    #[test]
    fn an_unauthorized_context_cannot_be_sent() {
        let mut context = Context::select(
            vec![ContextFile {
                path: "a".to_owned(),
                text: "hello".to_owned(),
            }],
            &[],
        );
        assert!(context.sendable_payload().is_err());
        context.authorize();
        assert!(context.sendable_payload().is_ok());
    }

    #[test]
    fn a_change_set_refuses_the_generated_zone_and_a_stale_base() {
        let generated = ChangeSet {
            version: 1,
            base_revision: 4,
            changes: vec![change("src/generated/ui.rs", "old", "new")],
        };
        assert!(generated.validate().is_err());
        let target = ChangeSet {
            version: 1,
            base_revision: 4,
            changes: vec![change("target/debug/log", "old", "new")],
        };
        assert!(target.validate().is_err());
        let current = ChangeSet {
            version: 1,
            base_revision: 4,
            changes: vec![change("src/handwritten/app.rs", "old", "new")],
        };
        assert!(current.validate_against(5).is_err());
        assert!(current.validate_against(4).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_that_leaves_the_project_root_is_refused() {
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        );
        let root = std::env::temp_dir().join(format!("rustage-ai-root-{unique}"));
        let inside = root.join("src");
        std::fs::create_dir_all(&inside).expect("a project directory");
        let outside = std::env::temp_dir().join(format!("rustage-ai-outside-{unique}"));
        std::fs::create_dir_all(&outside).expect("an outside directory");
        std::os::unix::fs::symlink(&outside, inside.join("escape")).expect("a symlink");

        let escaping = ChangeSet {
            version: 1,
            base_revision: 1,
            changes: vec![change("src/escape/x.rs", "", "x")],
        };
        let error = escaping.validate_in(&root).expect_err("a refusal");
        assert!(error.contains("leaves the project root"), "{error}");

        let new_file = ChangeSet {
            version: 1,
            base_revision: 1,
            changes: vec![change("src/new.rs", "", "x")],
        };
        assert!(new_file.validate_in(&root).is_ok());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_candidate_digest_is_frozen_and_old_verification_is_refused() {
        let mut candidate = candidate();
        let digest = candidate.digest.clone();
        candidate
            .verify(&digest, Verification::Tested)
            .expect("a verify");
        assert_eq!(candidate.verification, Verification::Tested);
        assert!(candidate.verify("00", Verification::Compiled).is_err());
    }

    #[test]
    fn partial_acceptance_keeps_only_the_chosen_changes() {
        let candidate = candidate();
        let accepted = candidate.accept(&[1]).expect("a partial accept");
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].path, "b.rs");
        assert!(candidate.accept(&[9]).is_err());
    }

    #[test]
    fn applying_then_rolling_back_restores_the_previous_text() {
        let candidate = candidate();
        let accepted = candidate.accept(&[0]).expect("an accept");
        let mut workspace = Workspace::new(
            [("a.rs".to_owned(), "one".to_owned())]
                .into_iter()
                .collect::<BTreeMap<_, _>>(),
        );
        let undo = workspace.apply(&accepted).expect("an apply");
        assert_eq!(workspace.get("a.rs"), Some("two"));
        workspace.rollback(&undo);
        assert_eq!(workspace.get("a.rs"), Some("one"));
    }

    #[test]
    fn a_conflicted_base_refuses_an_apply() {
        let candidate = candidate();
        let accepted = candidate.accept(&[0]).expect("an accept");
        let mut workspace = Workspace::new(
            [("a.rs".to_owned(), "changed elsewhere".to_owned())]
                .into_iter()
                .collect(),
        );
        assert!(workspace.apply(&accepted).is_err());
    }

    #[test]
    fn roles_have_distinct_workspaces_and_a_bounded_tool_set() {
        let session = Session {
            role: Role::Reviewer,
            model: "reviewer".to_owned(),
            budget: 8,
            tools: ["read_file", "comment"]
                .into_iter()
                .map(str::to_owned)
                .collect::<BTreeSet<_>>(),
            credential: "env:REVIEWER_TOKEN".to_owned(),
            usage: Vec::new(),
        };
        assert_eq!(Role::Reviewer.directory(), ".ai/reviewer");
        assert!(session.allows("comment"));
        assert!(!session.allows("write_file"));
        let context = Context::select(
            vec![ContextFile {
                path: "a".to_owned(),
                text: "0123456789".to_owned(),
            }],
            &[],
        );
        assert!(!session.fits(&context));
    }

    #[test]
    fn a_session_ledger_totals_costs_and_keeps_broken_apart_from_cancelled() {
        let mut session = Session {
            role: Role::Coder,
            model: "coder".to_owned(),
            budget: 1024,
            tools: BTreeSet::new(),
            credential: String::new(),
            usage: Vec::new(),
        };
        for (elapsed, cost, outcome) in [
            (10, 5, InvocationOutcome::Completed),
            (20, 7, InvocationOutcome::Cancelled),
            (
                30,
                0,
                InvocationOutcome::Broken {
                    reason: "the stream ended mid-answer".to_owned(),
                },
            ),
        ] {
            session
                .record(UsageRecord {
                    model: "coder".to_owned(),
                    role: Role::Coder,
                    prompt_bytes: 100,
                    completion_bytes: 50,
                    cost_micros: cost,
                    elapsed_ms: elapsed,
                    outcome,
                })
                .expect("the record names this session's model");
        }
        let totals = session.totals();
        assert_eq!(totals.prompt_bytes, 300);
        assert_eq!(totals.completion_bytes, 150);
        assert_eq!(totals.cost_micros, 12);
        assert_eq!(totals.elapsed_ms, 60);
        assert_eq!(totals.completed, 1);
        assert_eq!(totals.cancelled, 1);
        assert_eq!(totals.broken, 1);
        assert!(totals.render().contains("3 call(s)"));
    }

    #[test]
    fn a_record_for_another_model_is_refused_rather_than_filed() {
        let mut session = Session {
            role: Role::Coder,
            model: "coder".to_owned(),
            budget: 1024,
            tools: BTreeSet::new(),
            credential: String::new(),
            usage: Vec::new(),
        };
        let error = session
            .record(UsageRecord {
                model: "other".to_owned(),
                role: Role::Coder,
                prompt_bytes: 1,
                completion_bytes: 1,
                cost_micros: 0,
                elapsed_ms: 1,
                outcome: InvocationOutcome::Completed,
            })
            .expect_err("a mixed ledger is refused");
        assert!(error.contains("other"), "{error}");
        assert!(session.usage().is_empty());
    }

    #[test]
    fn a_tool_call_is_previewed_only_for_an_allowlisted_tool_and_granted_per_call() {
        let mut session = Session {
            role: Role::Coder,
            model: "coder".to_owned(),
            budget: 1024,
            tools: ["read_file"].into_iter().map(str::to_owned).collect(),
            credential: String::new(),
            usage: Vec::new(),
        };
        assert!(
            session.preview_tool("write_file", "path=a.rs").is_err(),
            "a tool outside the allowlist is refused before authorization"
        );
        let call = session
            .preview_tool("read_file", "path=a.rs,token=abc")
            .expect("the tool is allowlisted");
        assert!(call.preview().contains("[redacted]"), "{}", call.preview());
        // MCP requires a JSON object for `arguments`; the non-JSON text above is still a valid
        // preview and authorization key, but it is not a sendable request body.
        assert!(call.mcp_json().is_err());
        let structured = session
            .preview_tool("read_file", "{\"path\":\"a.rs\",\"token\":\"abc\"}")
            .expect("the tool is allowlisted");
        let body = structured.mcp_json().expect("a JSON object is sendable");
        assert!(body.contains("\"method\":\"tools/call\""), "{body}");
        assert!(body.contains("read_file"), "{body}");

        let mut policy = ToolPolicy::default();
        assert!(!policy.decide(&call).allowed());
        policy.authorize(&call);
        assert!(policy.decide(&call).allowed());
        let other = session
            .preview_tool("read_file", "path=b.rs")
            .expect("the tool is allowlisted");
        assert!(
            !policy.decide(&other).allowed(),
            "a grant for one exact call must not cover another"
        );
        policy.clear();
        assert!(!policy.decide(&call).allowed());
        session
            .record(UsageRecord {
                model: "coder".to_owned(),
                role: Role::Coder,
                prompt_bytes: 0,
                completion_bytes: 0,
                cost_micros: 0,
                elapsed_ms: 0,
                outcome: InvocationOutcome::Completed,
            })
            .expect("the ledger still accepts a record");
    }

    #[test]
    fn a_handoff_is_bound_to_one_candidate_digest_and_revision() {
        let candidate = candidate();
        let handoff = Handoff::new(
            Role::Coder,
            Role::Reviewer,
            &candidate,
            vec![".ai/share/brief.md".to_owned()],
        )
        .expect("two roles can hand over");
        assert!(Handoff::new(Role::Coder, Role::Coder, &candidate, Vec::new()).is_err());
        handoff
            .check(&candidate, candidate.changes.base_revision)
            .expect("the same candidate and revision are valid");
        let conflict = handoff
            .check(&candidate, candidate.changes.base_revision + 1)
            .expect_err("a moved document is a conflict");
        assert!(conflict.contains("revision"), "{conflict}");
        assert_eq!(handoff.cleanup(), &[".ai/share/brief.md".to_owned()]);

        let other = Candidate::new_at(
            2,
            ChangeSet {
                version: 1,
                base_revision: 4,
                changes: vec![change("c.rs", "", "x")],
            },
            4,
        )
        .expect("another candidate");
        let mismatch = handoff
            .check(&other, 4)
            .expect_err("a different candidate is refused");
        assert!(mismatch.contains("candidate"), "{mismatch}");
    }
}
