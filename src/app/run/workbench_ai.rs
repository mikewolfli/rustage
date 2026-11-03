// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The AI-session panel's rows (roadmap F23, `panel.ai`).
//!
//! The panel is a **read-only view over the upstream journal** (`ai::journal`), selected with
//! `--ai-journal <path>`. It starts no agent, sends no request and writes no candidate: the journal
//! is the durable record a person or an external agent produced elsewhere, and this panel shows
//! exactly what it holds.
//!
//! **Boundary.** A journal stores the role, model, budget, tool allowlist, credential *reference*,
//! usage ledger, candidates and handoffs. It does not store the authorized context selection, so
//! the panel says that fact is not in the journal rather than inventing one.

use std::path::Path;

use crate::app::ai::Journal;

/// The rows for one journal file.
pub(super) fn rows(path: &Path) -> Vec<String> {
    let journal = match Journal::load(path) {
        Ok(journal) => journal,
        Err(reason) => {
            return vec![format!("journal {}: {reason}", path.display())];
        }
    };
    let session = journal.session();
    let restored = session.restore();
    let mut rows = vec![format!(
        "journal: {} (sequence {})",
        path.display(),
        journal.seq()
    )];
    rows.push(format!(
        "role: {:?}; model: {}; budget: {} byte(s)",
        session.role, session.model, session.budget
    ));
    rows.push(format!(
        "tools: {}",
        if session.tools.is_empty() {
            "(none)".to_owned()
        } else {
            session.tools.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    ));
    rows.push(format!(
        "credential reference: {}",
        if session.credential.trim().is_empty() {
            "(none)".to_owned()
        } else {
            session.credential.clone()
        }
    ));
    rows.push(format!("usage: {}", restored.totals().render()));

    rows.push(format!("candidates: {}", journal.candidates().len()));
    for candidate in journal.candidates() {
        rows.push(format!(
            "  candidate {}: base r{}, {} change(s), verification {:?}, {} comment(s)",
            candidate.id,
            candidate.changes.base_revision,
            candidate.changes.changes.len(),
            candidate.verification,
            candidate.comments.len()
        ));
    }
    rows.push(format!("handoffs: {}", journal.handoffs().len()));
    for handoff in journal.handoffs() {
        rows.push(format!(
            "  handoff {:?} -> {:?}: candidate {}, base r{}, {} shared path(s)",
            handoff.from,
            handoff.to,
            handoff.candidate,
            handoff.base_revision,
            handoff.shared.len()
        ));
    }
    rows.push("authorized context: not recorded in the journal".to_owned());
    rows.push("actions: not run; this panel starts no agent and writes no candidate".to_owned());
    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::ai::{Journal, Role, SessionRecord};
    use std::collections::BTreeSet;

    fn journal(path: &std::path::Path) {
        let mut journal = Journal::new(SessionRecord {
            role: Role::Coder,
            model: "coder-model".to_owned(),
            budget: 4096,
            tools: BTreeSet::from(["read_file".to_owned()]),
            credential: "env:AI_KEY".to_owned(),
            usage: Vec::new(),
        });
        journal.save(path).expect("the journal is written");
    }

    #[test]
    fn a_journal_renders_its_session_and_the_facts_it_does_not_hold() {
        let path = std::env::temp_dir().join(format!(
            "rustage-ai-journal-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        journal(&path);
        let rows = rows(&path);
        let _ = std::fs::remove_file(&path);
        assert!(rows[0].starts_with("journal: "), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("Coder")));
        assert!(rows.iter().any(|row| row.contains("coder-model")));
        assert!(rows.iter().any(|row| row.contains("read_file")));
        assert!(rows.iter().any(|row| row.contains("env:AI_KEY")));
        assert!(
            rows.iter()
                .any(|row| row.contains("authorized context: not recorded"))
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("starts no agent and writes no candidate"))
        );
    }

    #[test]
    fn a_missing_journal_is_a_row_not_an_empty_success() {
        let path = std::env::temp_dir().join("rustage-ai-journal-missing-does-not-exist.json");
        let rows = rows(&path);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].starts_with("journal "), "{rows:?}");
        assert!(!rows[0].is_empty());
    }
}
