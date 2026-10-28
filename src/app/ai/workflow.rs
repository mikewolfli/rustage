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
}

#[cfg(test)]
mod tests {
    use super::{
        Candidate, Change, ChangeSet, Context, ContextFile, Role, Session, Verification, Workspace,
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
}
