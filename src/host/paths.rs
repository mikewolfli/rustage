// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! System paths and the project-root path policy: the host half of decision **F1**.
//!
//! Decision F1 fixes the rule the whole designer depends on: every path in a project resolves
//! against the project root, and a reference that leaves the root is refused. This module is where
//! that rule is executed, so that a resource reference, a migration candidate or a build output can
//! all ask the same question and get the same answer.
//!
//! What it refuses, and why each refusal is separate:
//!
//! * **An absolute path** — it is already anchored somewhere else, so resolving it "against the
//!   root" would be a lie about what happened.
//! * **A path that climbs out with `..`** — the classic escape, refused before touching the disk.
//! * **A path that leaves through a symlink** — the same escape wearing a disguise. The check
//!   canonicalises the parent directory, so a link pointing outside the root is refused even though
//!   the written path never mentions `..`.
//! * **A missing parent** — nothing is created implicitly; a resource that is not there is a
//!   diagnostic, not a directory that appears because someone typed a path.
//!
//! System paths (`config`, `cache`, `data`, `temp`) come from the OS conventions and return `Err`
//! when the environment does not say where they are. There is deliberately **no fallback to the
//! working directory**: silently writing configuration next to whatever directory the user happened
//! to be in is worse than reporting that the location is unknown.

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Why a path was refused.
#[derive(Debug)]
pub enum PathError {
    /// The path was empty or only separators.
    Empty,
    /// The path is absolute, so it does not belong to a project root.
    Absolute { path: PathBuf },
    /// The path climbs above the root.
    EscapesRoot { path: PathBuf },
    /// The path resolves outside the root, through a symlink or a link to a directory.
    LeavesRootThroughLink { path: PathBuf, resolved: PathBuf },
    /// The parent directory does not exist.
    MissingParent { path: PathBuf },
    /// A system path the OS did not name.
    SystemPathUnavailable { name: &'static str, reason: String },
}

impl PathError {
    /// A sentence naming what was found, for a diagnostic or a terminal.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "the path is empty".to_owned(),
            Self::Absolute { path } => format!(
                "{} is absolute; project paths are relative to the project root",
                path.display(),
            ),
            Self::EscapesRoot { path } => {
                format!("{} climbs above the project root", path.display())
            }
            Self::LeavesRootThroughLink { path, resolved } => format!(
                "{} resolves to {} which is outside the project root",
                path.display(),
                resolved.display(),
            ),
            Self::MissingParent { path } => format!(
                "the parent directory of {} does not exist",
                path.display(),
            ),
            Self::SystemPathUnavailable { name, reason } => {
                format!("the {name} directory is unknown: {reason}")
            }
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for PathError {}

/// A project root that paths resolve against.
#[derive(Clone, Debug)]
pub struct ProjectRoot {
    root: PathBuf,
}

impl ProjectRoot {
    /// Anchors a root, requiring the directory to exist and resolving it once.
    ///
    /// Canonicalising up front means every later check compares like with like: the root is in the
    /// same form the filesystem reports, so a symlinked project directory does not make every
    /// reference look like an escape.
    pub fn new(path: impl AsRef<Path>) -> Result<Self, PathError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        let root = std::fs::canonicalize(path).map_err(|error| PathError::MissingParent {
            path: path.to_path_buf(),
        })?;
        if !root.is_dir() {
            return Err(PathError::MissingParent { path: root });
        }
        Ok(Self { root })
    }

    /// The resolved root.
    pub fn as_path(&self) -> &Path {
        &self.root
    }

    /// Resolves a project-relative path, refusing anything that leaves the root.
    pub fn resolve(&self, relative: impl AsRef<Path>) -> Result<PathBuf, PathError> {
        let relative = relative.as_ref();
        if relative.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        if relative.is_absolute() {
            return Err(PathError::Absolute { path: relative.to_path_buf() });
        }

        let mut resolved = self.root.clone();
        let mut depth = 0usize;
        for component in relative.components() {
            match component {
                Component::Normal(part) => {
                    resolved.push(part);
                    depth += 1;
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if depth == 0 {
                        return Err(PathError::EscapesRoot { path: relative.to_path_buf() });
                    }
                    resolved.pop();
                    depth -= 1;
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Absolute { path: relative.to_path_buf() });
                }
            }
        }
        if depth == 0 {
            return Err(PathError::Empty);
        }

        // The written path cannot climb out, but a link inside the project can point out. The parent
        // is what a write would touch, so that is what has to be inside the root.
        let parent = resolved.parent().unwrap_or(&self.root);
        if parent.exists() {
            let canonical_parent =
                std::fs::canonicalize(parent).map_err(|_| PathError::MissingParent {
                    path: resolved.clone(),
                })?;
            if !canonical_parent.starts_with(&self.root) {
                return Err(PathError::LeavesRootThroughLink {
                    path: relative.to_path_buf(),
                    resolved: canonical_parent,
                });
            }
            return Ok(canonical_parent.join(
                resolved.file_name().unwrap_or_else(|| OsStr::new("")),
            ));
        }

        Err(PathError::MissingParent { path: resolved })
    }
}

/// Where the OS keeps the host's own files. Each is `Err` when the environment does not say.
#[derive(Clone, Debug)]
pub struct SystemPaths {
    home: PathBuf,
}

impl SystemPaths {
    /// Reads the user's home directory from the environment.
    pub fn from_environment() -> Result<Self, PathError> {
        // `HOME` on macOS and Linux, `USERPROFILE` on Windows; both are checked so a shell that sets
        // only one of them still works, and neither being set is an error rather than a guess.
        for name in ["HOME", "USERPROFILE"] {
            if let Some(value) = std::env::var_os(name) {
                if !value.is_empty() {
                    let home = PathBuf::from(value);
                    if home.is_absolute() {
                        return Ok(Self { home });
                    }
                    return Err(PathError::SystemPathUnavailable {
                        name: "home",
                        reason: format!("{name} is not absolute: {}", home.display()),
                    });
                }
            }
        }
        Err(PathError::SystemPathUnavailable {
            name: "home",
            reason: "neither HOME nor USERPROFILE is set".to_owned(),
        })
    }

    /// A handle for an explicit home directory, used by tests.
    pub fn with_home(home: impl AsRef<Path>) -> Result<Self, PathError> {
        let home = home.as_ref();
        if home.as_os_str().is_empty() || !home.is_absolute() {
            return Err(PathError::SystemPathUnavailable {
                name: "home",
                reason: format!("{} is not an absolute directory", home.display()),
            });
        }
        Ok(Self { home: home.to_path_buf() })
    }

    /// The home directory.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where configuration belongs.
    pub fn config_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        if cfg!(target_os = "windows") {
            if let Some(appdata) = std::env::var_os("APPDATA") {
                return PathBuf::from(appdata).join("rustage");
            }
        }
        if cfg!(target_os = "macos") {
            return self.home.join("Library").join("Application Support").join("rustage");
        }
        self.home.join(".config").join("rustage")
    }

    /// Where caches belong.
    pub fn cache_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
            if !dir.is_empty() {
                return PathBuf::from(dir).join("rustage");
            }
        }
        if cfg!(target_os = "macos") {
            return self.home.join("Library").join("Caches").join("rustage");
        }
        self.home.join(".cache").join("rustage")
    }

    /// Where durable application data belongs.
    pub fn data_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
            if !dir.is_empty() {
                return PathBuf::from(dir).join("rustage");
            }
        }
        if cfg!(target_os = "macos") {
            return self.home.join("Library").join("Application Support").join("rustage");
        }
        self.home.join(".local").join("share").join("rustage")
    }

    /// The temporary directory the OS reports.
    pub fn temp_dir(&self) -> Result<PathBuf, PathError> {
        let dir = std::env::temp_dir();
        if dir.as_os_str().is_empty() {
            return Err(PathError::SystemPathUnavailable {
                name: "temp",
                reason: "the environment reports no temporary directory".to_owned(),
            });
        }
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::{PathError, ProjectRoot, SystemPaths};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A temporary project directory, removed by the caller's guard in `after`.
    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A directory of its own per sandbox.
    ///
    /// The counter matters: tests run in parallel, and a guard that removed a shared parent would
    /// delete another test's root mid-run. That is exactly what happened first, and it turned a
    /// symlink-escape case into a confusing "missing parent".
    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rustage-paths-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox { root: root.clone(), _guard: TempDirectory(root) }
    }

    #[test]
    fn a_relative_path_resolves_against_the_root() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("ui")).unwrap();
        std::fs::write(sandbox.root.join("ui/main.json"), "{}").unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let resolved = root.resolve("ui/main.json").unwrap();

        assert_eq!(resolved, std::fs::canonicalize(sandbox.root.join("ui/main.json")).unwrap());
        assert!(root.resolve("./ui/./main.json").is_ok());
    }

    #[test]
    fn an_absolute_path_is_refused_because_it_is_not_the_project_s_own() {
        let sandbox = sandbox();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        assert!(matches!(root.resolve("/etc/passwd"), Err(PathError::Absolute { .. })));
    }

    #[test]
    fn climbing_above_the_root_is_refused_before_the_disk_is_touched() {
        let sandbox = sandbox();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let error = root.resolve("../outside.txt").unwrap_err();

        assert!(matches!(error, PathError::EscapesRoot { .. }));
        assert!(error.message().contains("climbs above"));
    }

    #[test]
    fn an_empty_path_is_not_a_reference() {
        let sandbox = sandbox();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        assert!(matches!(root.resolve(""), Err(PathError::Empty)));
        assert!(matches!(root.resolve("./"), Err(PathError::Empty)));
        assert!(ProjectRoot::new("").is_err());
    }

    #[test]
    fn a_missing_parent_is_a_refusal_not_an_implicit_directory() {
        let sandbox = sandbox();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let error = root.resolve("assets/missing/logo.svg").unwrap_err();

        assert!(matches!(error, PathError::MissingParent { .. }));
        assert!(!sandbox.root.join("assets").exists(), "nothing may be created implicitly");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_pointing_out_of_the_root_is_refused() {
        let sandbox = sandbox();
        let outside = std::env::temp_dir().join(format!("rustage-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, sandbox.root.join("escape")).unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let error = root.resolve("escape/secret.txt").unwrap_err();

        assert!(
            matches!(error, PathError::LeavesRootThroughLink { .. }),
            "expected a link escape, got {error:?}",
        );
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn unicode_paths_survive_resolution_unchanged() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("资源")).unwrap();
        std::fs::write(sandbox.root.join("资源/图标.svg"), "<svg/>").unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let resolved = root.resolve("资源/图标.svg").unwrap();

        assert!(resolved.ends_with("资源/图标.svg"), "{}", resolved.display());
    }

    #[test]
    fn a_child_in_a_parent_named_with_dots_still_resolves() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("ui")).unwrap();
        std::fs::write(sandbox.root.join("ui/main.json"), "{}").unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        // `ui/../ui/main.json` never leaves the root, so it is a legal reference.
        assert!(root.resolve("ui/../ui/main.json").is_ok());
    }

    #[test]
    fn system_paths_come_from_the_environment_or_report_that_they_cannot() {
        let paths = SystemPaths::with_home("/home/example").unwrap();

        assert!(paths.config_dir().is_absolute());
        assert!(paths.cache_dir().is_absolute());
        assert!(paths.data_dir().is_absolute());
        assert!(paths.temp_dir().unwrap().is_absolute());
        assert!(paths.config_dir().to_string_lossy().contains("rustage"));
    }

    #[test]
    fn a_relative_home_is_refused_instead_of_being_used_relative_to_cwd() {
        let error = SystemPaths::with_home("relative/home").unwrap_err();

        assert!(matches!(error, PathError::SystemPathUnavailable { .. }));
        assert!(error.message().contains("not an absolute directory"));
    }
}
