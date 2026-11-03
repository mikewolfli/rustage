// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The notifications and recovery panel's rows (roadmap F01/F03, `panel.notifications`).
//!
//! This is a view over the host's [`crate::host::notify::NotificationCentre`], and it adds nothing
//! to it: a row is [`crate::host::notify::Notification::render`] of one open notification, oldest
//! first, so a reader sees the problem that has been waiting longest rather than the newest noise.
//! Dismissing a row addresses the same notification the rendered order named.
//!
//! **The boundary.** The centre has no producers in the workbench yet. Its [`crate::host::notify::Cause`]
//! is a value only a real producer can construct — a finished task record, a process outcome, a
//! spawn failure or a permission refusal — and the GUI's log lines are not causes, so they are
//! deliberately not turned into rows here. The panel is therefore legitimately empty until a real
//! task, process or permission cause is reported, and [`Notifications::rows`] does not invent a
//! row to fill it; the caller owns the empty-state string. An empty panel is an honest answer, not
//! a missing feature.
//!
//! **The centre is the real service.** De-duplication (a repeat increments `occurrences` instead of
//! adding a row), severities, links to the thing a notification is about, and the next step for
//! each cause all already live in [`crate::host::notify`]. This module only maps indices onto that
//! order and passes verdicts through, so the panel cannot disagree with the centre.

use crate::host::notify::NotificationCentre;

/// The notifications and recovery panel, backed by the host notification centre.
pub(super) struct Notifications {
    centre: NotificationCentre,
}

impl Notifications {
    /// An empty centre.
    pub(super) fn new() -> Self {
        Self {
            centre: NotificationCentre::new(),
        }
    }

    /// One row per open notification ([`crate::host::notify::Notification::render`]), oldest first.
    ///
    /// Dismissed notifications are not rows, and an empty centre produces no rows at all rather
    /// than a placeholder: the caller has an empty-state string.
    pub(super) fn rows(&self) -> Vec<String> {
        self.centre
            .open()
            .iter()
            .map(|notification| notification.render())
            .collect()
    }

    /// Error / warning / information counts of the centre.
    pub(super) fn counts(&self) -> crate::host::notify::Counts {
        self.centre.counts()
    }

    /// Dismisses the notification the row index names, answering whether one was dismissed.
    ///
    /// The index is read in the same [`crate::host::notify::NotificationCentre::open`] order
    /// [`Self::rows`] renders, so the row a person clicked is the notification that is cleared. An
    /// index past the end answers `false` instead of dismissing something else.
    pub(super) fn dismiss(&self, index: usize) -> bool {
        match self.centre.open().get(index) {
            Some(notification) => self.centre.dismiss(notification.id()),
            None => false,
        }
    }

    /// Reports a real cause through the centre and answers its de-duplication verdict.
    ///
    /// The cause is a value only a real producer can construct; the workbench reports a finished
    /// build task this way, so a repeat increments the existing notification instead of adding a
    /// row.
    pub(super) fn report(
        &self,
        cause: crate::host::notify::Cause,
    ) -> crate::host::notify::Reported {
        self.centre.report(cause)
    }
}

#[cfg(test)]
mod tests {
    use super::Notifications;

    #[test]
    fn a_new_centre_has_no_rows() {
        let notifications = Notifications::new();

        assert!(
            notifications.rows().is_empty(),
            "an empty centre renders no placeholder row: {:?}",
            notifications.rows(),
        );
        assert!(notifications.counts().is_empty());
    }

    #[test]
    fn an_out_of_range_dismiss_is_false() {
        let notifications = Notifications::new();
        assert!(!notifications.dismiss(0), "there is no row to dismiss");
    }
}
