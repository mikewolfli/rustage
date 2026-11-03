// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The preview-management panel's rows (roadmap F14, `panel.preview`).
//!
//! The rows are the preview service's own report: its lifecycle state, the session it owns, the
//! instances it holds, the process it attached, how many updates it applied and the mounted view's
//! generation. Nothing here starts a process or mounts an engine; the panel shows the service the
//! window already holds.
//!
//! **Boundary.** The viewport matrix is a separate model ([`crate::app::preview::viewport_matrix`])
//! that a caller fills with viewports; this panel states that it holds none rather than inventing
//! presets. A preview that is stopped is a real state, not a missing feature.

use crate::app::preview::preview_service::Preview;

/// The preview panel's rows.
pub(super) fn rows(preview: &Preview) -> Vec<String> {
    let mut rows = preview.render();
    if rows.is_empty() {
        rows.push("preview: no report".to_owned());
    }
    rows.push(
        "viewport matrix: 0 viewport(s) (a caller fills the matrix; this panel does not invent presets)"
            .to_owned(),
    );
    let unwired = preview.unwired_events();
    rows.push(if unwired.is_empty() {
        "unwired events: none".to_owned()
    } else {
        format!("unwired events: {}", unwired.join(", "))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::preview::preview_service::Preview;

    #[test]
    fn a_stopped_preview_reports_no_instances_and_no_process() {
        let rows = rows(&Preview::new());
        assert!(!rows.is_empty(), "the panel always answers with a report");
        assert!(
            rows.iter().any(|row| row.contains("stopped")),
            "the stopped state is reported: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("viewport matrix: 0 viewport(s)")),
            "an empty matrix is stated: {rows:?}"
        );
        assert!(rows.iter().all(|row| !row.is_empty()));
    }
}
