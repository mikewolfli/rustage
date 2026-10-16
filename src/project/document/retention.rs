// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The history retention policy, and the joint failure it has to survive (task T03.08).
//!
//! The acceptance line is *"no unresolved data loss, and the retention policy never deletes a project
//! file"*. Both are about what a prune is **allowed to touch**, so the policy is written as a rule
//! about paths rather than as a list of files:
//!
//! * **The only things a prune may remove live in the recovery store.** Every candidate is checked
//!   against [`RecoveryStore::base`] before it is offered; a path that is not inside the store is
//!   refused and reported rather than removed. The store lives outside every project (T03.06), so
//!   "never a project file" follows from "only ever inside the store" — and the check is what makes
//!   the second true even if a caller hands in something odd.
//! * **The newest copies of a document are kept.** A policy keeps the newest `keep` copies per
//!   document; older ones are candidates.
//! * **A copy that is the only version is never a candidate.** If the document is gone
//!   ([`Standing::OnlyCopy`]), the copy is the last version there is, and no count may prune it. This
//!   is the rule that turns "no unresolved data loss" from a hope into a property: the policy cannot
//!   forget a document that no longer exists on disk.
//!
//! The joint failure test at the bottom is what the task asks for in its own words: a save that fails
//! while a recovery copy and a document both exist, followed by a prune, leaves the project file
//! byte-identical and the only-version copy in place.

use std::path::{Path, PathBuf};

use crate::project::document::recovery::{Copy, RecoveryStore, Standing};

/// How much history to keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy {
    keep_per_document: usize,
}

impl Policy {
    /// A policy that keeps the newest `keep_per_document` copies of each document.
    pub const fn new(keep_per_document: usize) -> Self {
        Self { keep_per_document }
    }

    /// The number of copies kept per document.
    pub const fn keep_per_document(self) -> usize {
        self.keep_per_document
    }
}

impl Default for Policy {
    /// Five copies per document: enough to step back through a bad afternoon, few enough that the
    /// store does not grow without bound.
    fn default() -> Self {
        Self::new(5)
    }
}

/// What a prune would do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    store: PathBuf,
    prune: Vec<PathBuf>,
    keep: Vec<PathBuf>,
    refused: Vec<String>,
}

impl Plan {
    /// The files that would be removed.
    pub fn prune(&self) -> &[PathBuf] {
        &self.prune
    }

    /// The files that would be kept.
    pub fn keep(&self) -> &[PathBuf] {
        &self.keep
    }

    /// Paths that were not inside the store, and so were refused and kept.
    pub fn refused(&self) -> &[String] {
        &self.refused
    }

    /// Whether there is nothing to remove.
    pub fn is_empty(&self) -> bool {
        self.prune.is_empty()
    }

    /// The plan in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "retention under {}: {} to remove, {} to keep",
            self.store.display(),
            self.prune.len(),
            self.keep.len()
        )];
        for path in &self.keep {
            lines.push(format!("  keep   {}", path.display()));
        }
        for path in &self.prune {
            lines.push(format!("  remove {}", path.display()));
        }
        for reason in &self.refused {
            lines.push(format!("  refused {reason}"));
        }
        lines
    }
}

/// Plans a prune of a store under a policy.
///
/// The grouping is by `(project root, document)`, in the store's own newest-first order, so "the
/// newest `keep`" is the same set a person sees in the startup-recovery report.
pub fn plan(store: &RecoveryStore, policy: Policy) -> Plan {
    let base = store.base().to_path_buf();
    let mut prune = Vec::new();
    let mut keep = Vec::new();
    let mut refused = Vec::new();

    let mut groups: Vec<((PathBuf, String), Vec<Copy>)> = Vec::new();
    for copy in store.all() {
        let key = (copy.root().to_path_buf(), copy.document().to_owned());
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, group)) => group.push(copy),
            None => groups.push((key, vec![copy])),
        }
    }

    for (_, group) in groups {
        for (rank, copy) in group.into_iter().enumerate() {
            let keep_this = rank < policy.keep_per_document()
                // The last version of a document that is gone is not a candidate, whatever the count.
                || copy.standing() == Standing::OnlyCopy;
            if keep_this {
                keep.push(copy.file().to_path_buf());
                continue;
            }
            // The rule that makes "never a project file" true: only paths inside the store may go.
            if copy.file().starts_with(&base) {
                prune.push(copy.file().to_path_buf());
            } else {
                refused.push(format!(
                    "{} is not inside the store, so it is kept",
                    copy.file().display()
                ));
                keep.push(copy.file().to_path_buf());
            }
        }
    }

    Plan {
        store: base,
        prune,
        keep,
        refused,
    }
}

/// What a prune removed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pruned {
    removed: Vec<PathBuf>,
    failed: Vec<String>,
}

impl Pruned {
    /// The files that were removed.
    pub fn removed(&self) -> &[PathBuf] {
        &self.removed
    }

    /// The files that could not be removed, with the OS's reason.
    pub fn failed(&self) -> &[String] {
        &self.failed
    }

    /// The prune in one sentence.
    pub fn message(&self) -> String {
        format!(
            "removed {} recovery copy/copies; {}{}",
            self.removed.len(),
            if self.failed.is_empty() {
                "none failed"
            } else {
                "some could not be removed: "
            },
            self.failed.join("; ")
        )
    }
}

/// Carries out a plan, removing only what the plan names.
pub fn apply(plan: &Plan) -> Pruned {
    let mut removed = Vec::new();
    let mut failed = Vec::new();
    for path in &plan.prune {
        // Re-check at the moment of removal: a plan is a description, and the rule about paths must
        // hold where it matters.
        if !path.starts_with(&plan.store) {
            failed.push(format!(
                "{} is not inside the store; nothing removed",
                path.display()
            ));
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => removed.push(path.clone()),
            Err(error) => failed.push(format!("{}: {error}", path.display())),
        }
    }
    Pruned { removed, failed }
}

/// Whether a path is inside the store, which is the whole of what a retention policy may touch.
pub fn is_within(store: &RecoveryStore, path: &Path) -> bool {
    path.starts_with(store.base())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::document::save::{Saving, Verb};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        project: crate::host::paths::ProjectRoot,
        project_path: PathBuf,
        store_path: PathBuf,
        _guard: TempDirectory,
    }

    impl Sandbox {
        fn store(&self) -> RecoveryStore {
            RecoveryStore::new(&self.store_path)
        }
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("rustage-retention-{}-{id}", std::process::id()));
        let project = base.join("project");
        std::fs::create_dir_all(project.join("pages")).expect("a directory");
        std::fs::write(project.join("project.json"), "{\"version\":1}").expect("a project");
        std::fs::write(project.join("pages/home.json"), "disk\n").expect("writable");
        let root = crate::host::paths::ProjectRoot::new(&project).expect("a root");
        Sandbox {
            project: root,
            project_path: project,
            store_path: base.join("store"),
            _guard: TempDirectory(base),
        }
    }

    fn at(seconds: u64) -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn the_policy_keeps_the_newest_copies_and_offers_the_older_ones() {
        let sandbox = sandbox();
        let store = sandbox.store();
        for second in 1..=7 {
            store
                .record(
                    &second.to_string(),
                    sandbox.project.as_path(),
                    "pages/home.json",
                    "disk\n",
                    at(second),
                )
                .expect("written");
        }
        let plan = plan(&store, Policy::new(3));
        assert_eq!(plan.keep().len(), 3);
        assert_eq!(plan.prune().len(), 4);
        // Newest three kept: sessions 7, 6, 5.
        for kept in plan.keep() {
            let name = kept.to_string_lossy();
            assert!(
                name.contains("/5/") || name.contains("/6/") || name.contains("/7/"),
                "{name}"
            );
        }
    }

    #[test]
    fn a_copy_that_is_the_only_version_is_never_a_candidate() {
        let sandbox = sandbox();
        let store = sandbox.store();
        let file = sandbox.project_path.join("pages/home.json");
        // One copy per session; the file changes each time so only the newest matches the disk.
        for second in 1..=7 {
            store
                .record(
                    &second.to_string(),
                    sandbox.project.as_path(),
                    "pages/home.json",
                    &format!("v{second}\n"),
                    at(second),
                )
                .expect("written");
        }
        // Now the document is gone: every copy is the only version, so none may be pruned.
        std::fs::remove_file(&file).expect("removable");
        let plan = plan(&store, Policy::new(1));
        assert!(plan.is_empty(), "{}", plan.render().join("\n"));
        assert_eq!(plan.keep().len(), 7);
    }

    #[test]
    fn a_prune_only_ever_names_files_inside_the_store() {
        let sandbox = sandbox();
        let store = sandbox.store();
        for second in 1..=4 {
            store
                .record(
                    &second.to_string(),
                    sandbox.project.as_path(),
                    "pages/home.json",
                    "disk\n",
                    at(second),
                )
                .expect("written");
        }
        let plan = plan(&store, Policy::new(1));
        assert!(!plan.is_empty());
        for path in plan.prune() {
            assert!(
                is_within(&store, path),
                "{} is outside the store",
                path.display()
            );
            assert!(
                !path.starts_with(sandbox.project_path.as_path()),
                "a project file was offered for pruning"
            );
        }
        assert!(plan.refused().is_empty());
    }

    #[test]
    fn applying_a_prune_removes_only_what_the_plan_named() {
        let sandbox = sandbox();
        let store = sandbox.store();
        for second in 1..=4 {
            store
                .record(
                    &second.to_string(),
                    sandbox.project.as_path(),
                    "pages/home.json",
                    "disk\n",
                    at(second),
                )
                .expect("written");
        }
        let before = store.all().len();
        let plan = plan(&store, Policy::new(2));
        let pruned = apply(&plan);
        assert_eq!(pruned.removed().len(), 2);
        assert!(pruned.failed().is_empty());
        assert_eq!(store.all().len(), before - 2);
        assert_eq!(
            std::fs::read_to_string(sandbox.project_path.join("pages/home.json")).unwrap(),
            "disk\n",
            "the project file is never touched by a prune"
        );
    }

    /// The joint failure T03.08 asks for: a save fails while a recovery copy exists, and a prune
    /// afterwards still leaves the project file byte-identical and the only version in place.
    #[cfg(unix)]
    #[test]
    fn a_failed_save_a_kept_recovery_and_a_prune_lose_nothing() {
        use std::os::unix::fs::PermissionsExt;
        if is_root() {
            return;
        }
        let sandbox = sandbox();
        let store = sandbox.store();
        let file = sandbox.project_path.join("pages/home.json");
        let original = std::fs::read(&file).expect("readable");

        // A recovery copy from an earlier session, and one more so there is something to prune.
        store
            .record(
                "1",
                sandbox.project.as_path(),
                "pages/home.json",
                "earlier\n",
                at(1),
            )
            .expect("written");
        store
            .record(
                "2",
                sandbox.project.as_path(),
                "pages/home.json",
                "disk\n",
                at(2),
            )
            .expect("written");

        // A save that fails: the directory refuses writes.
        std::fs::set_permissions(
            sandbox.project_path.join("pages"),
            std::fs::Permissions::from_mode(0o555),
        )
        .expect("chmod");
        let saving = Saving::plan(&sandbox.project, "pages/home.json", "edited\n", Verb::Save)
            .expect("a plan");
        let failure = saving.execute().expect_err("the write is refused");
        std::fs::set_permissions(
            sandbox.project_path.join("pages"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("chmod back");
        assert!(failure.original_intact(), "{}", failure.message());

        // Nothing was lost: the file is what it was, and both copies are still there.
        assert_eq!(std::fs::read(&file).expect("readable"), original);
        assert_eq!(store.all().len(), 2);

        // A prune now removes the older copy (the document still exists, so only the newest count
        // matters), and still leaves the file alone.
        let plan = plan(&store, Policy::new(1));
        assert_eq!(plan.prune().len(), 1);
        apply(&plan);
        assert_eq!(std::fs::read(&file).expect("readable"), original);
        assert_eq!(store.all().len(), 1);
    }

    #[cfg(unix)]
    fn is_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
            == Some(0)
    }
}
