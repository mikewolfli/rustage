// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Committing generated files, and what a committed file needs to build (task T15.07).
//!
//! The acceptance line is *"a failed generation keeps the old files, and the result does not depend
//! on the host running"*. Both are made true by what this module does and does not do:
//!
//! * **Writes land in two phases, and a failure puts the old files back.** [`commit`] writes every
//!   artifact to a temporary beside its destination first, and only when all of them are written does
//!   it move them into place. An old file is kept beside its destination before being replaced, so a
//!   later rename that fails restores every file already renamed and removes every file already
//!   created — the same discipline [`crate::project::document::save`] uses for one document, applied
//!   to a set.
//! * **Only a file this tool wrote may be replaced.** Every existing destination goes through
//!   [`super::merge::prepare`] first (T15.04): an unmarked file is refused, a marked file is merged,
//!   and a hand-written region inside it survives. A refusal happens while staging, so nothing has
//!   been renamed when it is reported.
//! * **Unchanged files are not touched.** A file whose merged result already equals what generation
//!   produced is reported [`Action::Unchanged`] and not rewritten, so its modification time does not
//!   move and a downstream build stays incremental.
//! * **The standalone contract is stated, not implied.** [`standalone_contract`] lists what the
//!   generated source needs — the pinned widget crate, one `--features <profile>` selection, no host
//!   process — so a reader can check a delivered file without the designer.

#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::merge;

/// One file to write: where it goes (project-relative) and what it holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Artifact {
    /// The project-relative destination.
    pub path: String,
    /// The file's content.
    pub source: String,
}

/// What committing an artifact did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// The file did not exist and was written.
    Created,
    /// The file existed with different content and was replaced.
    Updated,
    /// The file already had exactly this content, so nothing was written.
    Unchanged,
}

impl Action {
    /// The action in one line.
    pub fn render(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

/// One artifact and what committing it did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Committed {
    /// The destination, relative to the project root.
    pub path: String,
    /// What happened to it.
    pub action: Action,
}

impl Committed {
    /// The result in one line.
    pub fn render(&self) -> String {
        format!("{} {}", self.action.render(), self.path)
    }
}

/// One artifact staged for the second phase.
struct Staged {
    /// The project-relative destination, as the plan spells it.
    relative: String,
    /// Where it goes.
    destination: PathBuf,
    /// The temporary holding the new bytes, `None` when the file is unchanged.
    temporary: Option<PathBuf>,
    /// What the write does to the destination.
    action: Action,
}

/// A rename that already happened, and the old file it replaced.
struct Applied {
    /// The destination now holding the new bytes.
    destination: PathBuf,
    /// The old file kept beside it, `None` when the destination did not exist before.
    backup: Option<PathBuf>,
}

/// Writes every artifact through a temporary, then renames the whole set into place.
///
/// A destination outside `root` is refused before anything is written, because a generated file that
/// escapes the project is not a generated file the project can build. An existing destination must be
/// a generated file whose hand-written boundaries are provable; it is merged, not overwritten. When a
/// write or a rename fails, every temporary is removed, every rename already made is undone, and no
/// destination is left with new bytes, so a failed generation keeps the old project.
pub fn commit(root: &Path, artifacts: &[Artifact]) -> Result<Vec<Committed>, String> {
    let mut staged: Vec<Staged> = Vec::with_capacity(artifacts.len());
    let mut seen: BTreeSet<&str> = BTreeSet::new();

    for artifact in artifacts {
        let relative = Path::new(&artifact.path);
        if relative.is_absolute() || artifact.path.contains("..") {
            cleanup(&staged);
            return Err(format!(
                "generated output {} is not inside the project",
                artifact.path
            ));
        }
        // Two artifacts writing one destination would have the second rename clobber the first, so
        // the shape is refused while nothing has been written rather than reported as a surprise.
        if !seen.insert(artifact.path.as_str()) {
            cleanup(&staged);
            return Err(format!(
                "generated output {} is listed twice",
                artifact.path
            ));
        }
        let destination = root.join(relative);
        let (action, text) = match prepare(&destination, &artifact.source) {
            Ok(prepared) => prepared,
            Err(reason) => {
                cleanup(&staged);
                return Err(format!("{}: {reason}", artifact.path));
            }
        };
        if action == Action::Unchanged {
            staged.push(Staged {
                relative: artifact.path.clone(),
                destination,
                temporary: None,
                action,
            });
            continue;
        }
        if let Some(parent) = destination.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            cleanup(&staged);
            return Err(format!(
                "{}: could not make the directory: {error}",
                artifact.path
            ));
        }
        let temporary = beside(&destination, ".tmp");
        if let Err(error) = std::fs::write(&temporary, &text) {
            let _ = std::fs::remove_file(&temporary);
            cleanup(&staged);
            return Err(format!("{}: could not write: {error}", artifact.path));
        }
        staged.push(Staged {
            relative: artifact.path.clone(),
            destination,
            temporary: Some(temporary),
            action,
        });
    }

    let mut applied: Vec<Applied> = Vec::new();
    for item in &staged {
        let Some(temporary) = &item.temporary else {
            continue;
        };
        // Keep the old file beside its destination before replacing it: a later failure can then put
        // it back exactly as it was rather than reconstructing it from memory.
        let backup = if item.action == Action::Updated {
            let backup = beside(&item.destination, ".bak");
            let _ = std::fs::remove_file(&backup);
            if let Err(error) = std::fs::rename(&item.destination, &backup) {
                let _ = std::fs::remove_file(temporary);
                rollback(&applied);
                cleanup(&staged);
                return Err(format!(
                    "{}: could not keep the old file: {error}",
                    item.relative
                ));
            }
            Some(backup)
        } else {
            None
        };
        if let Err(error) = std::fs::rename(temporary, &item.destination) {
            if let Some(backup) = &backup {
                let _ = std::fs::rename(backup, &item.destination);
            }
            let _ = std::fs::remove_file(temporary);
            rollback(&applied);
            cleanup(&staged);
            return Err(format!(
                "{}: could not move into place: {error}",
                item.relative
            ));
        }
        applied.push(Applied {
            destination: item.destination.clone(),
            backup,
        });
    }

    // Every rename succeeded: the old bytes are no longer needed.
    for item in &applied {
        if let Some(backup) = &item.backup {
            let _ = std::fs::remove_file(backup);
        }
    }

    Ok(staged
        .into_iter()
        .map(|item| Committed {
            path: item.relative,
            action: item.action,
        })
        .collect())
}

/// Decides what a destination needs, and the bytes that would go there.
///
/// A missing file is created. An existing file is merged through [`merge::prepare`], which refuses
/// one that is not ours and refuses a generated body whose hand-written boundaries cannot be proven.
/// The result is computed here, before any rename, so a refusal leaves every destination untouched.
fn prepare(destination: &Path, generated: &str) -> Result<(Action, String), String> {
    match std::fs::read_to_string(destination) {
        Ok(existing) => match merge::prepare(&existing, generated)? {
            merge::Prepared::Unchanged => Ok((Action::Unchanged, existing)),
            merge::Prepared::Write(text) => Ok((Action::Updated, text)),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((Action::Created, generated.to_owned()))
        }
        // A destination that exists but cannot be read as text is not a file this tool can merge
        // with. It is staged like a new file and the rename refuses it, which rolls the set back.
        Err(_) => Ok((Action::Created, generated.to_owned())),
    }
}

/// A path beside `destination` with `suffix` appended, keeping its extension.
fn beside(destination: &Path, suffix: &str) -> PathBuf {
    let mut name = destination.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Undoes the renames already made, newest first.
///
/// An updated destination gets its kept old file back; a created one is removed. The new bytes are
/// removed before the restore so the restore is the same operation on every platform.
fn rollback(applied: &[Applied]) {
    for item in applied.iter().rev() {
        let _ = std::fs::remove_file(&item.destination);
        if let Some(backup) = &item.backup {
            let _ = std::fs::rename(backup, &item.destination);
        }
    }
}

/// Removes the temporaries staged so far, on the failure path.
fn cleanup(staged: &[Staged]) {
    for item in staged {
        if let Some(temporary) = &item.temporary {
            let _ = std::fs::remove_file(temporary);
        }
    }
}

/// What a generated file needs to build, without the designer.
///
/// Stated as lines so a reader can compare a delivered file against them; each is true of the library
/// output this family emits, and none names the host process.
pub fn standalone_contract() -> Vec<String> {
    vec![
        "depends on the pinned rust-widgets revision and nothing else".to_owned(),
        "built with exactly one profile feature (`desktop`, `tablet`, `mobile`, `mini`, `embedded`)"
            .to_owned(),
        "the default template names `crate::view`; the stripped template does not".to_owned(),
        "written under `src/generated/`, one file per template".to_owned(),
        "holds no reference to the designer's process, port or preview session".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::{Action, Artifact, commit, standalone_contract};
    use crate::app::generate::artifact::FILE_MARKER;
    use std::path::PathBuf;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "rustage-generate-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temp directory");
            Self(path)
        }

        fn root(&self) -> &std::path::Path {
            &self.0
        }

        fn read(&self, path: &str) -> String {
            std::fs::read_to_string(self.0.join(path)).expect("the file")
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("the directory");
            }
            std::fs::write(path, text).expect("the file");
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A generated file, as the pipeline writes it: the library marker first.
    fn marked(body: &str) -> String {
        format!("{FILE_MARKER}\n{body}\n")
    }

    #[test]
    fn committing_writes_new_files_and_leaves_unchanged_ones_alone() {
        let temp = Temp::new("commit");
        let artifacts = vec![Artifact {
            path: "src/generated/ui.rs".to_owned(),
            source: marked("pub fn build_ui() {}"),
        }];
        let first = commit(temp.root(), &artifacts).expect("a commit");
        assert_eq!(first[0].action, Action::Created);
        assert!(temp.root().join("src/generated/ui.rs").is_file());

        let second = commit(temp.root(), &artifacts).expect("a commit");
        assert_eq!(second[0].action, Action::Unchanged);
        assert_eq!(temp.read("src/generated/ui.rs"), artifacts[0].source);
    }

    #[test]
    fn a_changed_artifact_is_updated() {
        let temp = Temp::new("update");
        commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/ui.rs".to_owned(),
                source: marked("one"),
            }],
        )
        .expect("a commit");
        let result = commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/ui.rs".to_owned(),
                source: marked("two"),
            }],
        )
        .expect("a commit");
        assert_eq!(result[0].action, Action::Updated);
        assert_eq!(temp.read("src/generated/ui.rs"), marked("two"));
    }

    #[test]
    fn an_output_that_leaves_the_project_is_refused_and_writes_nothing() {
        let temp = Temp::new("escape");
        let error = commit(
            temp.root(),
            &[Artifact {
                path: "../outside.rs".to_owned(),
                source: marked("x"),
            }],
        )
        .expect_err("an escaping output");
        assert!(error.contains("not inside the project"), "{error}");
        assert!(!temp.root().join("src").exists());
    }

    #[test]
    fn an_unmarked_existing_file_is_refused_before_anything_is_renamed() {
        let temp = Temp::new("unmarked");
        temp.write(
            "src/generated/mine.rs",
            "fn main() { println!(\"mine\"); }\n",
        );

        let error = commit(
            temp.root(),
            &[
                Artifact {
                    path: "src/generated/new.rs".to_owned(),
                    source: marked("new"),
                },
                Artifact {
                    path: "src/generated/mine.rs".to_owned(),
                    source: marked("theirs"),
                },
            ],
        )
        .expect_err("not a generated file");
        assert!(error.contains("generated marker"), "{error}");
        assert_eq!(
            temp.read("src/generated/mine.rs"),
            "fn main() { println!(\"mine\"); }\n",
            "a refused write must not touch the hand-written file"
        );
        assert!(
            !temp.root().join("src/generated/new.rs").exists(),
            "a refused set must not leave the earlier artifact behind"
        );
    }

    #[test]
    fn a_marked_file_is_merged_and_keeps_its_handwritten_region() {
        let temp = Temp::new("merge");
        commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/ui.rs".to_owned(),
                source: marked("let a = 1;"),
            }],
        )
        .expect("a commit");

        let with_region = format!(
            "{}// region:handwritten save\nfn save() {{ /* mine */ }}\n// endregion:handwritten save\n",
            temp.read("src/generated/ui.rs")
        );
        temp.write("src/generated/ui.rs", &with_region);

        let result = commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/ui.rs".to_owned(),
                source: marked("let a = 2;"),
            }],
        )
        .expect("a guarded commit");
        assert_eq!(result[0].action, Action::Updated);

        let after = temp.read("src/generated/ui.rs");
        assert!(after.contains("let a = 2;"), "{after}");
        assert!(after.contains("fn save() { /* mine */ }"), "{after}");
    }

    #[test]
    fn a_failure_on_a_later_file_rolls_back_the_earlier_renames() {
        let temp = Temp::new("rollback");
        let old = marked("old");
        commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/first.rs".to_owned(),
                source: old.clone(),
            }],
        )
        .expect("the first commit");
        // A directory where a file is planned makes the second rename fail after the first one has
        // already been renamed into place.
        std::fs::create_dir_all(temp.root().join("src/generated/blocked.rs")).expect("a directory");

        let error = commit(
            temp.root(),
            &[
                Artifact {
                    path: "src/generated/first.rs".to_owned(),
                    source: marked("new"),
                },
                Artifact {
                    path: "src/generated/blocked.rs".to_owned(),
                    source: marked("blocked"),
                },
            ],
        )
        .expect_err("a later file cannot be moved into place");
        assert!(error.contains("could not move into place"), "{error}");

        assert_eq!(
            temp.read("src/generated/first.rs"),
            old,
            "the earlier rename has to be undone"
        );
        assert!(
            !temp.root().join("src/generated/first.rs.bak").exists(),
            "the kept old file is removed once it is back in place"
        );
        assert!(
            !temp.root().join("src/generated/blocked.rs.tmp").exists(),
            "no temporary is left behind"
        );
    }

    #[test]
    fn the_standalone_contract_names_the_profile_and_disclaims_the_host() {
        let contract = standalone_contract();
        assert!(contract.iter().any(|line| line.contains("profile feature")));
        assert!(
            contract
                .iter()
                .any(|line| line.contains("no reference to the designer"))
        );
    }
}
