// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The extensions and tools panel's rows (roadmap F22, `panel.extensions`).
//!
//! The rows are the extension manager's own listing: id, version, licence, enabled/disabled and
//! pinned state, how much of what the extension provides the open project uses, and the source it
//! came from ([`crate::app::extension::manager::Panel::render`]). The panel owns no registry of its
//! own; an empty registry renders no rows and the caller owns the empty-state string.
//!
//! **Boundary.** There is no extension-install surface in the workbench yet, so the registry the
//! window passes is empty until one lands. Compatibility and authorisation are checked by the
//! registry's own install path, not re-judged here; logs stay in the host log.

use crate::app::extension::Registry;
use crate::app::extension::manager::{Panel, Usage};

/// One row per installed extension.
pub(super) fn rows(registry: &Registry, usage: &Usage) -> Vec<String> {
    Panel::new(registry, usage).render()
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::app::extension::Registry;
    use crate::app::extension::manager::Usage;

    #[test]
    fn an_empty_registry_renders_no_placeholder_row() {
        let registry = Registry::new();
        let usage = Usage::new();
        assert!(rows(&registry, &usage).is_empty());
    }
}
