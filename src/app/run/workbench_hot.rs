// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The hot-update panel's rows (roadmap F17, `panel.hot`).
//!
//! The rows are the updater's own queue and history: how many edits are pending, and one line per
//! flushed update with the kind the batch was classified as (`live` or `rebuild`) and the patch and
//! update counts the preview reported. Nothing here classifies an edit itself or invents an update.
//!
//! **Boundary.** The workbench does not yet route document edits into the updater, so the queue is
//! empty until it does; the panel says so instead of drawing a synthetic candidate.

use crate::app::preview::hot_update::Updater;

/// The most history rows one report carries.
const ROW_LIMIT: usize = 200;

/// The hot-update panel's rows.
pub(super) fn rows(updater: &Updater) -> Vec<String> {
    let pending = updater.pending();
    let mut rows = vec![format!("pending edits: {pending}")];
    let history = updater.history();
    if history.is_empty() {
        rows.push("no hot update has been applied".to_owned());
        return rows;
    }
    let start = history.len().saturating_sub(ROW_LIMIT);
    if start > 0 {
        rows.push(format!("… {start} earlier update(s) …"));
    }
    rows.extend(history[start..].iter().map(|update| update.render()));
    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::preview::hot_update::{Edit, Updater};

    #[test]
    fn a_fresh_updater_has_nothing_pending_and_no_history() {
        let rows = rows(&Updater::new("workbench"));
        assert_eq!(rows[0], "pending edits: 0");
        assert!(
            rows.iter()
                .any(|row| row == "no hot update has been applied"),
            "{rows:?}"
        );
        assert!(rows.iter().all(|row| !row.is_empty()));
    }

    #[test]
    fn a_structural_edit_is_classified_as_a_rebuild_by_the_updater() {
        let mut updater = Updater::new("workbench");
        assert!(updater.push("scene", Edit::AddNode("button".to_owned())));
        let update = updater.flush().expect("one edit was pending");
        assert_eq!(update.kind.name(), "rebuild");
        assert_eq!(update.edits[0].render(), "add-node button");
        let rows = rows(&updater);
        assert!(
            rows.iter()
                .any(|row| row.starts_with("rebuild update from scene")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_property_edit_stays_live() {
        let mut updater = Updater::new("workbench");
        assert!(updater.push("inspector", Edit::Property("title on 2".to_owned())));
        let update = updater.flush().expect("one edit was pending");
        assert_eq!(update.kind.name(), "live");
        assert!(!update.kind.needs_cargo());
    }
}
