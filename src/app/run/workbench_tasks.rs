// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The background-tasks panel's rows (roadmap F01, `panel.tasks`).
//!
//! The panel is a view over the host executor's own records: each row is
//! [`TaskRecord::summary`], so queued / running / cancelling / finished, the owner, progress and
//! how the task ended all come from the executor rather than from a second bookkeeping list.
//!
//! **Boundary.** Nothing in the workbench submits a background task yet, so the list is empty until
//! one lands; the caller owns the empty-state string. The executor's records are the truth: a task
//! that was asked to stop stays `cancelling` until the job returns, and a job that outlived its
//! deadline is reported as abandoned by the executor, not promoted here.

use crate::host::tasks::{Executor, ExecutorConfig};

/// The panel's view of the process's executor.
pub(super) struct Tasks {
    executor: Executor,
}

impl Tasks {
    /// Starts the executor the panel reads.
    ///
    /// The default configuration is a valid one (two workers and a bounded queue), so the only way
    /// this can fail is a defect in those constants.
    pub(super) fn new() -> Self {
        Self {
            executor: Executor::new(ExecutorConfig::default())
                .expect("the default executor configuration has workers and a queue"),
        }
    }

    /// One row per task the executor knows, in id order.
    pub(super) fn rows(&self) -> Vec<String> {
        self.executor
            .records()
            .into_iter()
            .map(|record| record.summary())
            .collect()
    }

    /// The executor this process submits work into.
    pub(super) fn executor(&self) -> &Executor {
        &self.executor
    }
}

#[cfg(test)]
mod tests {
    use super::Tasks;

    #[test]
    fn a_new_executor_has_no_task() {
        let tasks = Tasks::new();
        assert!(tasks.rows().is_empty(), "no task was submitted");
    }
}
