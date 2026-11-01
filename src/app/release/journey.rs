// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The release-candidate journey: freeze facts → stage → verify → install → uninstall
//! (task T25.05).
//!
//! This is a runner, not a claim: every step is the same function the individual commands use, so a
//! passing journey means the commands agree with each other. It stops at the first failure and
//! returns the step that failed, because a later step reading a half-written predecessor would
//! report a second failure that is really the first one.
//!
//! The journey deliberately creates a user project and a settings file after installing, and checks
//! both after uninstalling: "uninstall deletes no user project" is a T25.04 promise that only this
//! walk can prove end to end.

use std::path::Path;

use super::{Freeze, install, maintenance, stage};

/// One journey step's result.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    /// What the step did.
    pub name: String,
    /// Its lines.
    pub lines: Vec<String>,
}

impl Step {
    /// The step and its lines, one per line.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("PASS {}", self.name)];
        lines.extend(self.lines.iter().map(|line| format!("  {line}")));
        lines
    }
}

/// Runs the whole journey in `workdir`, which must not already hold a `stage` or `install`
/// directory.
pub fn journey(
    binary: &Path,
    language_directory: &Path,
    workdir: &Path,
    freeze: &Freeze,
) -> Result<Vec<Step>, String> {
    if workdir.join("stage").exists() || workdir.join("install").exists() {
        return Err(format!(
            "{} already holds a stage or an install; a journey starts in a clean directory",
            workdir.display()
        ));
    }
    std::fs::create_dir_all(workdir)
        .map_err(|error| format!("cannot create {}: {error}", workdir.display()))?;

    let mut steps = Vec::new();

    let staged = stage::stage(binary, language_directory, &workdir.join("stage"), freeze)
        .map_err(|reason| format!("stage: {reason}"))?;
    steps.push(Step {
        name: "stage".to_owned(),
        lines: staged
            .files
            .iter()
            .map(|file| format!("{} ({} byte(s)) {}", file.path, file.bytes, file.digest))
            .collect(),
    });

    let drill = maintenance::drill(&staged.directory)
        .map_err(|failures| format!("drill: {}", failures.join("; ")))?;
    steps.push(Step {
        name: "verify".to_owned(),
        lines: drill.render(),
    });

    let plan = install::plan(&staged.directory, &workdir.join("install"))
        .map_err(|reason| format!("plan: {reason}"))?;
    let installed = install::install(&plan).map_err(|reason| format!("install: {reason}"))?;
    steps.push(Step {
        name: "install".to_owned(),
        lines: installed,
    });

    // The user's own files, written after the install.
    let root = workdir.join("install");
    std::fs::write(root.join("project.json"), b"{}")
        .map_err(|error| format!("cannot write the user project: {error}"))?;
    std::fs::write(root.join("settings.json"), b"{}")
        .map_err(|error| format!("cannot write settings: {error}"))?;

    let removed = install::uninstall(&root).map_err(|reason| format!("uninstall: {reason}"))?;
    if !root.join("project.json").is_file() {
        return Err("uninstall deleted the user project".to_owned());
    }
    if !root.join("settings.json").is_file() {
        return Err("uninstall deleted the settings".to_owned());
    }
    if root.join("rustage").exists() {
        return Err("uninstall left the binary behind".to_owned());
    }
    let mut lines = removed;
    lines.push("a user project and settings survived the uninstall".to_owned());
    steps.push(Step {
        name: "uninstall".to_owned(),
        lines,
    });

    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::journey;
    use crate::app::release::Freeze;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-journey-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    fn freeze() -> Freeze {
        let mut freeze = Freeze::new();
        freeze.revision = "rev".to_owned();
        freeze.package_version = "0.1.0".to_owned();
        freeze.generator = "0.1.0".to_owned();
        freeze.widget_revision = "widgets".to_owned();
        freeze
    }

    #[test]
    fn a_clean_work_directory_passes_every_step() {
        let root = scratch("pass");
        let source = root.join("source");
        let language = source.join("language");
        std::fs::create_dir_all(&language).expect("a language directory");
        std::fs::write(source.join("rustage"), b"a binary").expect("a binary");
        std::fs::write(language.join("en.json"), b"{}").expect("a pack");
        let steps =
            journey(&source.join("rustage"), &language, &root, &freeze()).expect("a journey");
        assert_eq!(
            steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            vec!["stage", "verify", "install", "uninstall"]
        );
        assert!(Path::new(&root.join("install/project.json")).is_file());
    }

    #[test]
    fn a_work_directory_that_already_holds_a_stage_is_refused() {
        let root = scratch("dirty");
        std::fs::create_dir_all(root.join("stage")).expect("a stage directory");
        let error =
            journey(Path::new("/does/not/exist"), &root, &root, &freeze()).expect_err("a refusal");
        assert!(error.contains("starts in a clean directory"), "{error}");
    }
}
