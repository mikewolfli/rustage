// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Committing generated files, and what a committed file needs to build (task T15.07).
//!
//! The acceptance line is *"a failed generation keeps the old files, and the result does not depend
//! on the host running"*. Both are made true by what this module does and does not do:
//!
//! * **Writes land in two phases.** [`commit`] writes every artifact to a temporary beside its
//!   destination first, and only when all of them are written does it rename them into place. A
//!   failure while writing leaves every old file exactly as it was — the same discipline
//!   [`crate::project::document::save`] uses for one document, applied to a set.
//! * **Unchanged files are not touched.** A file whose content already equals what generation
//!   produced is reported [`Action::Unchanged`] and not rewritten, so its modification time does not
//!   move and a downstream build stays incremental.
//! * **The standalone contract is stated, not implied.** [`standalone_contract`] lists what the
//!   generated source needs — the pinned widget crate, one `--features <profile>` selection, no host
//!   process — so a reader can check a delivered file without the designer.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

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

/// Writes every artifact through a temporary, then renames the whole set into place.
///
/// A destination outside `root` is refused before anything is written, because a generated file that
/// escapes the project is not a generated file the project can build. When a write fails, every
/// temporary is removed and no destination is touched, so a failed generation keeps the old files.
pub fn commit(root: &Path, artifacts: &[Artifact]) -> Result<Vec<Committed>, String> {
    let mut staged: Vec<(PathBuf, PathBuf, Action)> = Vec::new();

    for artifact in artifacts {
        let relative = Path::new(&artifact.path);
        if relative.is_absolute() || artifact.path.contains("..") {
            cleanup(&staged);
            return Err(format!(
                "generated output {} is not inside the project",
                artifact.path
            ));
        }
        let destination = root.join(relative);
        let action = match std::fs::read_to_string(&destination) {
            Ok(existing) if existing == artifact.source => Action::Unchanged,
            Ok(_) => Action::Updated,
            Err(_) => Action::Created,
        };
        if action == Action::Unchanged {
            staged.push((destination.clone(), destination, action));
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
        let temporary = destination.with_extension("rs.tmp");
        if let Err(error) = std::fs::write(&temporary, &artifact.source) {
            let _ = std::fs::remove_file(&temporary);
            cleanup(&staged);
            return Err(format!("{}: could not write: {error}", artifact.path));
        }
        staged.push((temporary, destination, action));
    }

    let mut out = Vec::with_capacity(staged.len());
    for (temporary, destination, action) in &staged {
        if *action != Action::Unchanged
            && let Err(error) = std::fs::rename(temporary, destination)
        {
            let _ = std::fs::remove_file(temporary);
            return Err(format!(
                "{}: could not move into place: {error}",
                destination.display()
            ));
        }
        out.push(Committed {
            path: artifacts
                .iter()
                .find(|artifact| root.join(&artifact.path) == *destination)
                .map(|artifact| artifact.path.clone())
                .unwrap_or_else(|| destination.display().to_string()),
            action: *action,
        });
    }
    Ok(out)
}

/// Removes the temporaries staged so far, on the failure path.
fn cleanup(staged: &[(PathBuf, PathBuf, Action)]) {
    for (temporary, _, action) in staged {
        if *action != Action::Unchanged {
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
    use std::path::PathBuf;

    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("rustage-generate-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temp directory");
            Self(path)
        }

        fn root(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn committing_writes_new_files_and_leaves_unchanged_ones_alone() {
        let temp = Temp::new("commit");
        let artifacts = vec![Artifact {
            path: "src/generated/ui.rs".to_owned(),
            source: "// generated\n".to_owned(),
        }];
        let first = commit(temp.root(), &artifacts).expect("a commit");
        assert_eq!(first[0].action, Action::Created);
        assert!(temp.root().join("src/generated/ui.rs").is_file());

        let second = commit(temp.root(), &artifacts).expect("a commit");
        assert_eq!(second[0].action, Action::Unchanged);
    }

    #[test]
    fn a_changed_artifact_is_updated() {
        let temp = Temp::new("update");
        commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/ui.rs".to_owned(),
                source: "one\n".to_owned(),
            }],
        )
        .expect("a commit");
        let result = commit(
            temp.root(),
            &[Artifact {
                path: "src/generated/ui.rs".to_owned(),
                source: "two\n".to_owned(),
            }],
        )
        .expect("a commit");
        assert_eq!(result[0].action, Action::Updated);
        assert_eq!(
            std::fs::read_to_string(temp.root().join("src/generated/ui.rs")).unwrap(),
            "two\n"
        );
    }

    #[test]
    fn an_output_that_leaves_the_project_is_refused_and_writes_nothing() {
        let temp = Temp::new("escape");
        let error = commit(
            temp.root(),
            &[Artifact {
                path: "../outside.rs".to_owned(),
                source: "x\n".to_owned(),
            }],
        )
        .expect_err("an escaping output");
        assert!(error.contains("not inside the project"), "{error}");
        assert!(!temp.root().join("src").exists());
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
