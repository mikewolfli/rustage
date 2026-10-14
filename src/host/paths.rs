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
    /// A path exists on paper but could not be resolved to the filesystem's own spelling.
    ///
    /// This keeps the OS's reason instead of reporting every resolution failure as "the parent is
    /// missing", which was wrong for a permission refusal and for a path whose links loop.
    Unresolved { path: PathBuf, reason: String },
    /// A directory that had to be a directory is something else.
    NotADirectory { path: PathBuf },
    /// A directory could not be created.
    NotCreated { path: PathBuf, reason: String },
    /// A directory exists but could not be listed.
    Unreadable { path: PathBuf, reason: String },
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
            Self::MissingParent { path } => {
                format!("the parent directory of {} does not exist", path.display(),)
            }
            Self::SystemPathUnavailable { name, reason } => {
                format!("the {name} directory is unknown: {reason}")
            }
            Self::Unresolved { path, reason } => {
                format!("{} could not be resolved: {reason}", path.display())
            }
            Self::NotADirectory { path } => {
                format!("{} is not a directory", path.display())
            }
            Self::NotCreated { path, reason } => {
                format!("{} could not be created: {reason}", path.display())
            }
            Self::Unreadable { path, reason } => {
                format!("{} could not be read: {reason}", path.display())
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
        let root = std::fs::canonicalize(path).map_err(|error| PathError::Unresolved {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
        if !root.is_dir() {
            return Err(PathError::NotADirectory { path: root });
        }
        Ok(Self { root })
    }

    /// Resolves a project-relative path that is going to be created.
    ///
    /// The same rule as [`Self::resolve`] about what stays inside the root, with one difference: the
    /// path's own directory does not have to exist yet. Creating a file in a new directory is
    /// ordinary, and a caller that has to make the directory anyway must not be told that the path
    /// "climbs above the root".
    ///
    /// The link check walks up to the nearest directory that does exist and canonicalises that one,
    /// so a link inside the project still cannot be used to write outside it.
    pub fn resolve_for_creation(&self, relative: impl AsRef<Path>) -> Result<PathBuf, PathError> {
        let relative = relative.as_ref();
        if relative.as_os_str().is_empty() {
            return Err(PathError::Empty);
        }
        if relative.is_absolute() {
            return Err(PathError::Absolute {
                path: relative.to_path_buf(),
            });
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
                        return Err(PathError::EscapesRoot {
                            path: relative.to_path_buf(),
                        });
                    }
                    resolved.pop();
                    depth -= 1;
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Absolute {
                        path: relative.to_path_buf(),
                    });
                }
            }
        }
        if depth == 0 {
            return Err(PathError::Empty);
        }

        // Walk up to the nearest directory that exists, and check that one: a link inside the project
        // that points out of it is caught here, and a directory nobody has made yet is not.
        let mut existing = resolved.as_path();
        while let Some(parent) = existing.parent() {
            existing = parent;
            if existing.exists() || existing == self.root {
                break;
            }
        }
        if existing.exists() {
            let canonical = std::fs::canonicalize(existing).map_err(|_| PathError::Unresolved {
                path: existing.to_path_buf(),
                reason: "it could not be resolved to the filesystem's own spelling".to_owned(),
            })?;
            if !canonical.starts_with(&self.root) && !self.root.starts_with(&canonical) {
                return Err(PathError::LeavesRootThroughLink {
                    path: relative.to_path_buf(),
                    resolved: canonical,
                });
            }
        }
        Ok(resolved)
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
            return Err(PathError::Absolute {
                path: relative.to_path_buf(),
            });
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
                        return Err(PathError::EscapesRoot {
                            path: relative.to_path_buf(),
                        });
                    }
                    resolved.pop();
                    depth -= 1;
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(PathError::Absolute {
                        path: relative.to_path_buf(),
                    });
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
            return Ok(
                canonical_parent.join(resolved.file_name().unwrap_or_else(|| OsStr::new("")))
            );
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
            if let Some(value) = std::env::var_os(name)
                && !value.is_empty()
            {
                {
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
        Ok(Self {
            home: home.to_path_buf(),
        })
    }

    /// The home directory.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where configuration belongs.
    pub fn config_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
            && !dir.is_empty()
        {
            return PathBuf::from(dir);
        }
        if cfg!(target_os = "windows")
            && let Some(appdata) = std::env::var_os("APPDATA")
        {
            return PathBuf::from(appdata).join("rustage");
        }
        if cfg!(target_os = "macos") {
            return self
                .home
                .join("Library")
                .join("Application Support")
                .join("rustage");
        }
        self.home.join(".config").join("rustage")
    }

    /// Where caches belong.
    pub fn cache_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_CACHE_HOME")
            && !dir.is_empty()
        {
            return PathBuf::from(dir).join("rustage");
        }
        if cfg!(target_os = "macos") {
            return self.home.join("Library").join("Caches").join("rustage");
        }
        self.home.join(".cache").join("rustage")
    }

    /// Where durable application data belongs.
    pub fn data_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("XDG_DATA_HOME")
            && !dir.is_empty()
        {
            return PathBuf::from(dir).join("rustage");
        }
        if cfg!(target_os = "macos") {
            return self
                .home
                .join("Library")
                .join("Application Support")
                .join("rustage");
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

/// One of the four places the host keeps its own files.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostDirectory {
    /// Public configuration: settings, the decision-visible options.
    Config,
    /// Caches that may be deleted at any time.
    Cache,
    /// Durable data the host must not lose.
    Data,
    /// Scratch space for in-flight work.
    Temp,
}

impl HostDirectory {
    /// The four directories, in the order a report should list them.
    pub const ALL: [Self; 4] = [Self::Config, Self::Cache, Self::Data, Self::Temp];

    /// The name this directory is known by in messages.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Cache => "cache",
            Self::Data => "data",
            Self::Temp => "temp",
        }
    }
}

/// What the host found at a host directory, without changing it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DirectoryState {
    /// The directory is there and can be listed.
    Usable { path: PathBuf },
    /// The directory is not there yet. Not a failure: [`HostDirectories::ensure`] creates it.
    Absent { path: PathBuf },
    /// Something else is in the way.
    NotADirectory { path: PathBuf },
    /// The directory is not there and cannot be created, or cannot be listed. The reason is the
    /// OS's own, because "the cache directory failed" is not a diagnosis.
    Unusable { path: PathBuf, reason: String },
}

impl DirectoryState {
    /// The path this state is about.
    pub fn path(&self) -> &Path {
        match self {
            Self::Usable { path } | Self::Absent { path } | Self::NotADirectory { path } => path,
            Self::Unusable { path, .. } => path,
        }
    }

    /// Whether the directory is there and readable.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Usable { .. })
    }

    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::Usable { path } => format!("{} is available", path.display()),
            Self::Absent { path } => format!("{} does not exist yet", path.display()),
            Self::NotADirectory { path } => format!("{} is not a directory", path.display()),
            Self::Unusable { path, reason } => {
                format!("{} cannot be used: {reason}", path.display())
            }
        }
    }
}

/// The host directories the process will actually use, resolved once.
///
/// Resolved eagerly rather than re-read from the environment at each call, because a host that asks
/// twice can get two answers: an environment edited between the read and the write would put half
/// the host's files in one place and half in another. One instance, one answer (principle #6).
#[derive(Clone, Debug)]
pub struct HostDirectories {
    config: PathBuf,
    cache: PathBuf,
    data: PathBuf,
    temp: PathBuf,
}

impl HostDirectories {
    /// Resolves the four directories from the operating system's conventions.
    pub fn from_environment() -> Result<Self, PathError> {
        let paths = SystemPaths::from_environment()?;
        Ok(Self {
            config: paths.config_dir(),
            cache: paths.cache_dir(),
            data: paths.data_dir(),
            temp: paths.temp_dir()?,
        })
    }

    /// Resolves the four directories from explicitly named paths.
    ///
    /// Used by the host's own checks and by tests: a check that wrote into the developer's real
    /// cache directory to prove the cache directory works would be a side effect nobody asked for.
    pub fn with_roots(
        config: impl Into<PathBuf>,
        cache: impl Into<PathBuf>,
        data: impl Into<PathBuf>,
        temp: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config: config.into(),
            cache: cache.into(),
            data: data.into(),
            temp: temp.into(),
        }
    }

    /// The path of one host directory.
    pub fn path(&self, directory: HostDirectory) -> &Path {
        match directory {
            HostDirectory::Config => &self.config,
            HostDirectory::Cache => &self.cache,
            HostDirectory::Data => &self.data,
            HostDirectory::Temp => &self.temp,
        }
    }

    /// Reports what is at one host directory without creating anything.
    pub fn inspect(&self, directory: HostDirectory) -> DirectoryState {
        let path = self.path(directory).to_path_buf();
        match std::fs::metadata(&path) {
            Ok(metadata) if !metadata.is_dir() => DirectoryState::NotADirectory { path },
            Ok(_) => match std::fs::read_dir(&path) {
                Ok(_) => DirectoryState::Usable { path },
                Err(error) => DirectoryState::Unusable {
                    path,
                    reason: error.to_string(),
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DirectoryState::Absent { path }
            }
            Err(error) => DirectoryState::Unusable {
                path,
                reason: error.to_string(),
            },
        }
    }

    /// Makes one host directory usable, creating it when it is missing, and returns its path.
    ///
    /// Creation is followed by a read-back, because `create_dir_all` succeeding is a claim about
    /// the call and not about the directory: a permission bit changed in between, or a filesystem
    /// that accepts the call and refuses the listing, would otherwise be reported as ready.
    pub fn ensure(&self, directory: HostDirectory) -> Result<&Path, PathError> {
        let path = self.path(directory);
        match self.inspect(directory) {
            DirectoryState::Usable { .. } => return Ok(path),
            DirectoryState::NotADirectory { path } => {
                return Err(PathError::NotADirectory { path });
            }
            DirectoryState::Absent { .. } | DirectoryState::Unusable { .. } => {}
        }

        if let Err(error) = std::fs::create_dir_all(path) {
            // An `Absent` that becomes `Unusable` between the two calls is why it is read again
            // rather than reported from the create error alone.
            return match self.inspect(directory) {
                DirectoryState::Usable { .. } => Ok(path),
                DirectoryState::NotADirectory { path } => Err(PathError::NotADirectory { path }),
                DirectoryState::Absent { path } | DirectoryState::Unusable { path, .. } => {
                    Err(PathError::NotCreated {
                        path,
                        reason: error.to_string(),
                    })
                }
            };
        }

        match self.inspect(directory) {
            DirectoryState::Usable { .. } => Ok(path),
            DirectoryState::NotADirectory { path } => Err(PathError::NotADirectory { path }),
            DirectoryState::Absent { path } => Err(PathError::NotCreated {
                path,
                reason: "the directory was created but is not there afterwards".to_owned(),
            }),
            DirectoryState::Unusable { path, reason } => {
                Err(PathError::Unreadable { path, reason })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DirectoryState, HostDirectories, HostDirectory, PathError, ProjectRoot, SystemPaths,
    };
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
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    /// A directory outside every sandbox, one per test.
    ///
    /// The same counter argument as [`sandbox`]: two tests that shared one directory would remove it
    /// from under each other, and a link left pointing at a directory that no longer exists is a
    /// different case from a link pointing outside the root.
    fn outside() -> TempDirectory {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rustage-outside-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a temporary directory is creatable");
        TempDirectory(path)
    }

    #[test]
    fn a_relative_path_resolves_against_the_root() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("ui")).unwrap();
        std::fs::write(sandbox.root.join("ui/main.json"), "{}").unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let resolved = root.resolve("ui/main.json").unwrap();

        assert_eq!(
            resolved,
            std::fs::canonicalize(sandbox.root.join("ui/main.json")).unwrap()
        );
        assert!(root.resolve("./ui/./main.json").is_ok());
    }

    #[test]
    fn an_absolute_path_is_refused_because_it_is_not_the_project_s_own() {
        let sandbox = sandbox();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        assert!(matches!(
            root.resolve("/etc/passwd"),
            Err(PathError::Absolute { .. })
        ));
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
        assert!(
            !sandbox.root.join("assets").exists(),
            "nothing may be created implicitly"
        );
    }

    #[test]
    fn a_path_that_is_going_to_be_created_may_be_in_a_new_directory() {
        let sandbox = sandbox();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        // The same path `resolve` refuses, because it is about to be created: the caller has to make
        // the directory anyway, and being told the path "climbs above the root" would be a lie.
        let target = root
            .resolve_for_creation("assets/new/logo.svg")
            .expect("a path in a directory that does not exist yet resolves");
        assert_eq!(target, sandbox.root.join("assets/new/logo.svg"));
        assert!(root.resolve("assets/new/logo.svg").is_err());
        assert!(
            !sandbox.root.join("assets").exists(),
            "resolving a path may not create anything"
        );

        // Everything else is the same rule: nothing empty, nothing absolute, nothing above the root.
        assert!(matches!(
            root.resolve_for_creation("../outside.svg"),
            Err(PathError::EscapesRoot { .. })
        ));
        assert!(matches!(
            root.resolve_for_creation("/tmp/outside.svg"),
            Err(PathError::Absolute { .. })
        ));
        assert!(matches!(
            root.resolve_for_creation(""),
            Err(PathError::Empty)
        ));
        assert!(matches!(
            root.resolve_for_creation("assets/.."),
            Err(PathError::Empty)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_new_directory_reached_through_a_link_out_of_the_root_is_still_refused() {
        let sandbox = sandbox();
        let outside = outside();
        std::os::unix::fs::symlink(&outside.0, sandbox.root.join("escape")).unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        // The link itself exists, so the check stops there and sees where it leads; the part of the
        // path that does not exist yet cannot be used to hide that.
        let error = root
            .resolve_for_creation("escape/new/secret.txt")
            .unwrap_err();
        assert!(
            matches!(error, PathError::LeavesRootThroughLink { .. }),
            "expected a link escape, got {error:?}",
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_pointing_out_of_the_root_is_refused() {
        let sandbox = sandbox();
        let outside = outside();
        std::os::unix::fs::symlink(&outside.0, sandbox.root.join("escape")).unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let error = root.resolve("escape/secret.txt").unwrap_err();

        assert!(
            matches!(error, PathError::LeavesRootThroughLink { .. }),
            "expected a link escape, got {error:?}",
        );
    }

    #[test]
    fn unicode_paths_survive_resolution_unchanged() {
        let sandbox = sandbox();
        std::fs::create_dir_all(sandbox.root.join("资源")).unwrap();
        std::fs::write(sandbox.root.join("资源/图标.svg"), "<svg/>").unwrap();
        let root = ProjectRoot::new(&sandbox.root).unwrap();

        let resolved = root.resolve("资源/图标.svg").unwrap();

        assert!(
            resolved.ends_with("资源/图标.svg"),
            "{}",
            resolved.display()
        );
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

    #[test]
    fn a_root_that_cannot_be_resolved_names_the_reason_it_could_not() {
        let missing = std::env::temp_dir().join("rustage-no-such-root-4f2a");

        let error = ProjectRoot::new(&missing).unwrap_err();

        // The old answer was "the parent directory does not exist" for every resolution failure,
        // which is a guess. The reason now comes from the OS.
        match error {
            PathError::Unresolved { path, reason } => {
                assert_eq!(path, missing);
                assert!(!reason.is_empty());
            }
            other => panic!("expected Unresolved, got {other:?}"),
        }
        assert!(!missing.exists(), "resolving a root must not create it");
    }

    #[test]
    fn a_root_that_is_a_file_is_refused_as_not_a_directory() {
        let sandbox = sandbox();
        let file = sandbox.root.join("not-a-root");
        std::fs::write(&file, "x").unwrap();

        assert!(matches!(
            ProjectRoot::new(&file),
            Err(PathError::NotADirectory { .. })
        ));
    }

    /// The four host directories, all inside one sandbox.
    fn sandbox_directories(sandbox: &Sandbox) -> HostDirectories {
        HostDirectories::with_roots(
            sandbox.root.join("config"),
            sandbox.root.join("cache"),
            sandbox.root.join("data"),
            sandbox.root.join("temp"),
        )
    }

    #[test]
    fn a_host_directory_that_is_absent_is_reported_as_absent_and_then_created() {
        let sandbox = sandbox();
        let directories = sandbox_directories(&sandbox);

        for directory in HostDirectory::ALL {
            let state = directories.inspect(directory);
            assert!(
                matches!(state, DirectoryState::Absent { .. }),
                "{} should be absent first, got {state:?}",
                directory.name(),
            );
            assert!(!state.is_usable());

            let ensured = directories
                .ensure(directory)
                .expect("creatable inside the sandbox");
            assert!(ensured.is_dir(), "{} was not created", directory.name());
            assert!(directories.inspect(directory).is_usable());
            assert!(
                directories.path(directory).ends_with(directory.name()),
                "{} resolved to {}",
                directory.name(),
                directories.path(directory).display(),
            );
        }
    }

    #[test]
    fn a_file_where_a_directory_belongs_is_reported_rather_than_overwritten() {
        let sandbox = sandbox();
        let directories = sandbox_directories(&sandbox);
        std::fs::write(directories.path(HostDirectory::Cache), "not a directory").unwrap();

        let state = directories.inspect(HostDirectory::Cache);

        assert_eq!(
            state,
            DirectoryState::NotADirectory {
                path: directories.path(HostDirectory::Cache).to_path_buf()
            }
        );
        let error = directories.ensure(HostDirectory::Cache).unwrap_err();
        assert!(matches!(error, PathError::NotADirectory { .. }));
        assert!(
            std::fs::read_to_string(directories.path(HostDirectory::Cache))
                .is_ok_and(|text| text == "not a directory"),
            "the file in the way must be left exactly as it was",
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_cannot_be_created_names_permission_as_the_reason() {
        use std::os::unix::fs::PermissionsExt;

        let sandbox = sandbox();
        let locked = sandbox.root.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
        permissions.set_mode(0o500);
        std::fs::set_permissions(&locked, permissions).unwrap();

        let directories =
            HostDirectories::with_roots(locked.join("config"), &locked, &locked, &locked);
        let result = directories.ensure(HostDirectory::Config);

        // A process running as root can write into a 0500 directory, so the assertion is about
        // what the host *reports*, not about which way the OS decided.
        match result {
            Err(PathError::NotCreated { path, reason }) => {
                assert!(path.starts_with(&locked));
                assert!(!reason.is_empty(), "a refusal must carry the OS's reason");
            }
            Err(other) => panic!("expected NotCreated, got {other:?}"),
            Ok(path) => {
                assert!(
                    path.is_dir(),
                    "if the OS allowed the creation, the host must report the real path"
                );
            }
        }

        let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
        permissions.set_mode(0o700);
        let _ = std::fs::set_permissions(&locked, permissions);
    }
}
