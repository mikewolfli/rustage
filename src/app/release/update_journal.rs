// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The persisted update journal (roadmap task T25.03).
//!
//! An update's state lived only in one process. That is enough for the state machine, but not for
//! the accident it exists for: a machine that dies between "the backup was made" and "the new files
//! are written" restarts with no memory of the interrupted install, and the next run would plan a
//! fresh update over a half-written one.
//!
//! This journal is that memory:
//!
//! * **Every transition is written.** [`Journal::write`] records the from/to versions, the artifact,
//!   the state and the backup name, so a later process can say what happened.
//! * **An `Installing` record is an interrupted install.** [`Journal::recover_interrupted`] turns it
//!   into `Interrupted` and answers the record, naming the backup. It does not pretend to restore
//!   files: this build's install is the state machine, and the journal says what the state was.
//! * **A missing journal is not a failure.** A machine that never updated has no file; that is
//!   `None`, not an error.
//! * **The file form is line-oriented and strict.** A record this module cannot read is refused with
//!   its line, because guessing the state of a half-applied update is worse than saying it cannot be
//!   read.

use std::path::{Path, PathBuf};

use crate::app::release::manifest::{Signature, Update, UpdateState};

/// One persisted update record.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    /// The version installed when the update was planned.
    pub from: String,
    /// The version the update moves to.
    pub to: String,
    /// The artifact's file name.
    pub artifact_name: String,
    /// The artifact's SHA-256.
    pub digest: String,
    /// The artifact's size in bytes.
    pub size: u64,
    /// Whether the artifact was signed.
    pub signed: bool,
    /// The last recorded state.
    pub state: UpdateState,
    /// The backup the update made, when it made one.
    pub backup: Option<String>,
}

impl Record {
    /// The record of an update's current state.
    pub fn of(update: &Update) -> Self {
        Self {
            from: update.from.clone(),
            to: update.to.clone(),
            artifact_name: update.artifact.name.clone(),
            digest: update.artifact.digest.clone(),
            size: update.artifact.size,
            signed: matches!(update.artifact.signature, Signature::Verified),
            state: update.state,
            backup: update.backup().map(str::to_owned),
        }
    }

    /// The record in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!("update: {} -> {}", self.from, self.to),
            format!(
                "artifact: {} {} byte(s), {}",
                self.artifact_name,
                self.size,
                if self.signed { "signed" } else { "unsigned" }
            ),
            format!("digest: {}", self.digest),
            format!("state: {}", self.state.name()),
        ];
        if let Some(backup) = &self.backup {
            lines.push(format!("backup: {backup}"));
        }
        lines
    }
}

/// The update journal at one path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Journal {
    path: PathBuf,
}

impl Journal {
    /// The journal at `path`.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Its path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the record, replacing the file.
    pub fn write(&self, record: &Record) -> Result<(), String> {
        let mut text = String::from("rustage-update 1\n");
        let mut field = |name: &str, value: &str| {
            text.push_str(name);
            text.push('\t');
            text.push_str(value);
            text.push('\n');
        };
        field("from", &record.from);
        field("to", &record.to);
        field("artifact", &record.artifact_name);
        field("digest", &record.digest);
        field("size", &record.size.to_string());
        field("signed", if record.signed { "yes" } else { "no" });
        field("state", record.state.name());
        if let Some(backup) = &record.backup {
            field("backup", backup);
        }
        std::fs::write(&self.path, text)
            .map_err(|error| format!("cannot write {}: {error}", self.path.display()))
    }

    /// Reads the record, answering `None` when the file does not exist.
    pub fn read(&self) -> Result<Option<Record>, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("cannot read {}: {error}", self.path.display()));
            }
        };
        let mut lines = text.lines();
        match lines.next() {
            Some(header) if header.trim() == "rustage-update 1" => {}
            Some(header) => {
                return Err(format!(
                    "{}: expected the `rustage-update 1` header, found {header:?}",
                    self.path.display()
                ));
            }
            None => {
                return Err(format!(
                    "{}: the update journal is empty",
                    self.path.display()
                ));
            }
        }
        let mut fields = std::collections::BTreeMap::new();
        for (index, line) in lines.enumerate() {
            if line.is_empty() {
                continue;
            }
            let number = index + 2;
            let Some((name, value)) = line.split_once('\t') else {
                return Err(format!(
                    "{}:{number}: a journal line is <name>\\t<value>",
                    self.path.display()
                ));
            };
            fields.insert(name.to_owned(), value.to_owned());
        }
        let take = |name: &str| -> Result<String, String> {
            fields
                .get(name)
                .cloned()
                .ok_or_else(|| format!("{}: the journal has no {name} field", self.path.display()))
        };
        let state = UpdateState::parse(&take("state")?).ok_or_else(|| {
            format!(
                "{}: {} is not an update state",
                self.path.display(),
                fields.get("state").map(String::as_str).unwrap_or("")
            )
        })?;
        let size: u64 = take("size")?
            .parse()
            .map_err(|_| format!("{}: the size is not a number", self.path.display()))?;
        let signed = match take("signed")?.as_str() {
            "yes" => true,
            "no" => false,
            other => {
                return Err(format!(
                    "{}: {other:?} is not yes or no",
                    self.path.display()
                ));
            }
        };
        Ok(Some(Record {
            from: take("from")?,
            to: take("to")?,
            artifact_name: take("artifact")?,
            digest: take("digest")?,
            size,
            signed,
            state,
            backup: fields.get("backup").cloned(),
        }))
    }

    /// Turns a recorded `Installing` state into `Interrupted` and answers it.
    ///
    /// `None` means there was no journal, or the last update was not in the middle of installing.
    /// The file is not touched in that case, so a finished install's record survives.
    pub fn recover_interrupted(&self) -> Result<Option<Record>, String> {
        let Some(mut record) = self.read()? else {
            return Ok(None);
        };
        if record.state != UpdateState::Installing {
            return Ok(None);
        }
        record.state = UpdateState::Interrupted;
        self.write(&record)?;
        Ok(Some(record))
    }

    /// Removes the journal.
    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("cannot remove {}: {error}", self.path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Journal, Record};
    use crate::app::release::Artifact;
    use crate::app::release::manifest::{Update, UpdateState};
    use sha2::{Digest, Sha256};

    const UPDATE_BYTES: &[u8] = b"release artifact bytes";

    fn update() -> Update {
        let mut hasher = Sha256::new();
        hasher.update(UPDATE_BYTES);
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let artifact = Artifact {
            name: "rustage-update".to_owned(),
            digest,
            size: UPDATE_BYTES.len() as u64,
            signature: crate::app::release::Signature::Unsigned,
            licenses: vec!["MIT".to_owned()],
        };
        let mut update = Update::plan("1.0.0", "1.1.0", artifact).expect("a plan");
        update.verify(UPDATE_BYTES).expect("a verified update");
        update.confirm().expect("a confirmed update");
        update
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rustage-journal-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("a directory");
        directory.join("update.journal")
    }

    #[test]
    fn a_record_round_trips_through_the_journal() {
        let path = scratch("roundtrip");
        let journal = Journal::at(&path);
        let record = Record::of(&update());
        journal.write(&record).expect("a write");
        assert_eq!(journal.read().expect("a read"), Some(record.clone()));
        assert!(record.render()[0].contains("1.0.0 -> 1.1.0"));
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn a_missing_journal_is_none_rather_than_an_error() {
        let path = scratch("missing");
        let journal = Journal::at(&path);
        assert_eq!(journal.read().expect("no journal"), None);
        assert_eq!(journal.recover_interrupted().expect("no recovery"), None);
        journal.clear().expect("clearing nothing is fine");
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn an_interrupted_install_is_marked_and_names_its_backup() {
        let path = scratch("interrupted");
        let journal = Journal::at(&path);
        let mut update = update();
        let backup = update.begin_install().expect("a backup");
        journal.write(&Record::of(&update)).expect("a write");

        let recovered = journal
            .recover_interrupted()
            .expect("a recovery")
            .expect("an interrupted record");
        assert_eq!(recovered.state, UpdateState::Interrupted);
        assert_eq!(recovered.backup.as_deref(), Some(backup.as_str()));
        // The change is persisted, so a second process sees the same answer.
        assert_eq!(
            journal.read().expect("a read").expect("a record").state,
            UpdateState::Interrupted
        );
        // And a second recovery does nothing, because it is no longer Installing.
        assert_eq!(journal.recover_interrupted().expect("no second"), None);
        assert_eq!(
            journal.read().expect("a read").expect("a record").state,
            UpdateState::Interrupted
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn a_finished_update_is_left_alone_by_recovery() {
        let path = scratch("installed");
        let journal = Journal::at(&path);
        let mut update = update();
        update.install().expect("an install");
        journal.write(&Record::of(&update)).expect("a write");
        assert_eq!(journal.recover_interrupted().expect("no recovery"), None);
        assert_eq!(
            journal.read().expect("a read").expect("a record").state,
            UpdateState::Installed
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn a_malformed_journal_is_refused_with_its_line() {
        let path = scratch("malformed");
        let journal = Journal::at(&path);
        std::fs::write(&path, "rustage-update 1\nfrom 1.0.0\n").expect("a file");
        let error = journal.read().expect_err("a malformed line");
        assert!(error.contains("line") || error.contains(":2:"), "{error}");

        std::fs::write(&path, "nonsense\n").expect("a file");
        assert!(journal.read().is_err());

        std::fs::write(&path, "rustage-update 1\nfrom\t1.0.0\nto\t1.1.0\n").expect("a file");
        let error = journal.read().expect_err("missing fields");
        assert!(
            error.contains("no artifact field") || error.contains("no state field"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }
}
