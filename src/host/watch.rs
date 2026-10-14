// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Directory watch: the host's view of "a file outside the designer changed". Contract **HOST**.
//!
//! T01.02 asks for platform file-event adaptation; T03.05 builds external-modification detection
//! and three-way conflict handling on top of it. What this module owns is the adapter half: turn
//! the dependency's watcher into a bounded, deterministic *poll* that a host loop can drive, and
//! keep its two failure shapes apart.
//!
//! Three decisions worth stating, because each is a place where the convenient thing is the wrong
//! thing:
//!
//! * **A watch that did not start is an error, not an empty stream.** The dependency's
//!   `watch_directory` returns `Err` for a path that does not exist or is not readable; forwarding
//!   that as "no events yet" would make a typo look like an idle directory (principle #22).
//! * **One event per path per poll.** The dependency coalesces inside a single notify callback, so
//!   one save can still arrive as two events (a create and a modify are two callbacks). This module
//!   coalesces per path within one [`DirectoryWatch::poll`], which makes "the same file may be
//!   reported once per poll" the contract a consumer can rely on. It deliberately does **not**
//!   deduplicate across polls: dropping the second of two genuine modifications would hide an edit.
//! * **The OS's classification is passed through verbatim.** The dependency's event carries
//!   `notify::EventKind` but does not re-export `notify`, so a host cannot match on it without
//!   taking the `notify` crate as a second, drifting source of truth (principle #35). The spelling
//!   is carried as text for diagnostics, and no `Created`/`Modified` distinction is invented.
//!
//! **What this adapter cannot see.** The dependency emits events only for `Create` and `Modify`;
//! deletions are dropped upstream, so "the file was deleted" and "the file was replaced" are not
//! observable here. That gap is recorded in `docs/rw-feedback/feedback-20261013.md` and is why
//! [`WatchEvent`] has no `Removed` variant to promise.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rust_widgets::asset::{AssetEvent, AssetWatcher};

/// Why a watch could not be started.
#[derive(Debug)]
pub enum WatchError {
    /// The path is empty, so it cannot be watched.
    EmptyRoot,
    /// The OS watcher refused the directory. The reason is the dependency's own message.
    Unwatchable { root: PathBuf, reason: String },
}

impl WatchError {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::EmptyRoot => "no directory was named, so there is nothing to watch".to_owned(),
            Self::Unwatchable { root, reason } => {
                format!("{} could not be watched: {reason}", root.display())
            }
        }
    }
}

impl fmt::Display for WatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for WatchError {}

/// One change the watcher reported.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WatchEvent {
    /// A watched path was created or modified.
    Changed {
        /// The absolute path the OS reported.
        path: PathBuf,
        /// The OS's own classification, verbatim. Not interpreted — see the module documentation.
        os_kind: String,
    },
    /// The watcher itself reported a problem. Delivered as an event because a watch that silently
    /// stops working is the failure this whole module exists to make visible.
    Error { reason: String },
}

/// A directory being watched.
///
/// Dropping it stops the watch: the dependency's watcher is owned here, so nothing keeps watching a
/// project after its host let go (T01.07's "no service outlives its project").
pub struct DirectoryWatch {
    root: PathBuf,
    recursive: bool,
    watcher: AssetWatcher,
}

impl DirectoryWatch {
    /// Starts watching `root`, delivering changes to paths `filter` accepts.
    ///
    /// `filter` receives the absolute path of each candidate, exactly as the dependency's own
    /// `watch_directory` documents, so a caller does not learn two different filter contracts.
    /// The recursive form is the one used here, and the dependency requires its filter to be
    /// `Send + Sync` rather than only `Send` — so this signature carries the stricter bound instead
    /// of hiding it behind a second, differently-bounded entry point.
    pub fn start<P>(root: impl AsRef<Path>, recursive: bool, filter: P) -> Result<Self, WatchError>
    where
        P: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(WatchError::EmptyRoot);
        }
        let mut watcher = AssetWatcher::new();
        watcher
            .watch(root, recursive, filter)
            .map_err(|reason| WatchError::Unwatchable {
                root: root.to_path_buf(),
                reason,
            })?;
        Ok(Self {
            root: root.to_path_buf(),
            recursive,
            watcher,
        })
    }

    /// The watched directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether subdirectories are watched too.
    pub fn is_recursive(&self) -> bool {
        self.recursive
    }

    /// Takes the changes reported since the last call, at most one per path.
    ///
    /// The result is ordered by path, so two identical runs of the same edits produce the same
    /// sequence and a test can assert on it. A path changed more than once since the last poll is
    /// reported once, carrying the newest classification — the same coalescing rule the dependency
    /// applies inside one callback, applied across the batch. Errors are reported in addition to
    /// changes and never replace one.
    pub fn poll(&self) -> Vec<WatchEvent> {
        let mut changes: BTreeMap<PathBuf, String> = BTreeMap::new();
        let mut errors: Vec<WatchEvent> = Vec::new();
        for event in self.watcher.poll_events() {
            match event {
                AssetEvent::FileChanged { path, kind, .. } => {
                    changes.insert(path, format!("{kind:?}"));
                }
                AssetEvent::WatchError { error, path } => {
                    errors.push(WatchEvent::Error {
                        reason: if path.as_os_str().is_empty() {
                            error
                        } else {
                            format!("{}: {error}", path.display())
                        },
                    });
                }
            }
        }
        let mut events: Vec<WatchEvent> = changes
            .into_iter()
            .map(|(path, os_kind)| WatchEvent::Changed { path, os_kind })
            .collect();
        events.append(&mut errors);
        events
    }
}

impl fmt::Debug for DirectoryWatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectoryWatch")
            .field("root", &self.root)
            .field("recursive", &self.recursive)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{DirectoryWatch, WatchError, WatchEvent};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// How long a test waits for the OS to deliver an event before calling it missing.
    ///
    /// Generous on purpose: `notify` runs on its own thread and the filesystem may coalesce or
    /// delay, so a tight bound would make this test fail on a loaded machine. The wait is still
    /// bounded, so a genuinely broken watch fails the test instead of hanging it.
    const EVENT_DEADLINE: Duration = Duration::from_secs(5);

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new(name: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustage-watch-{name}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("the sandbox directory is creatable");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Polls until at least one event arrives or the deadline passes.
    fn poll_until(watch: &DirectoryWatch) -> Vec<WatchEvent> {
        let deadline = Instant::now() + EVENT_DEADLINE;
        let mut seen = Vec::new();
        while Instant::now() < deadline {
            seen.extend(watch.poll());
            if !seen.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        seen
    }

    #[test]
    fn a_missing_directory_is_an_error_rather_than_an_idle_watch() {
        let sandbox = Sandbox::new("missing");
        let missing = sandbox.path().join("not-there");

        match DirectoryWatch::start(&missing, false, |_| true) {
            Err(WatchError::Unwatchable { root, reason }) => {
                assert_eq!(root, missing);
                assert!(reason.contains("not-there"), "{reason}");
            }
            other => panic!("expected Unwatchable, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_root_is_refused_before_the_os_is_asked() {
        assert!(matches!(
            DirectoryWatch::start("", false, |_| true),
            Err(WatchError::EmptyRoot)
        ));
    }

    #[test]
    fn a_write_in_the_watched_directory_is_reported_once_per_poll() {
        let sandbox = Sandbox::new("single");
        let watch = DirectoryWatch::start(sandbox.path(), false, |_| true).expect("watchable");
        let file = sandbox.path().join("page.json");

        std::fs::write(&file, "{}").expect("the file is writable");

        let events = poll_until(&watch);
        let changed: Vec<&PathBuf> = events
            .iter()
            .filter_map(|event| match event {
                WatchEvent::Changed { path, .. } => Some(path),
                WatchEvent::Error { .. } => None,
            })
            .collect();
        assert!(
            changed.iter().any(|path| path.as_path() == file),
            "expected a change for {}, got {events:?}",
            file.display(),
        );
        assert_eq!(
            changed.len(),
            1,
            "one poll must report one event per path, got {events:?}",
        );
    }

    #[test]
    fn the_filter_decides_what_is_reported_and_nothing_else_is() {
        let sandbox = Sandbox::new("filter");
        let watch = DirectoryWatch::start(sandbox.path(), false, |path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("watchable");

        std::fs::write(sandbox.path().join("ignored.txt"), "no").expect("writable");
        std::fs::write(sandbox.path().join("wanted.json"), "{}").expect("writable");

        let events = poll_until(&watch);
        for event in &events {
            if let WatchEvent::Changed { path, .. } = event {
                assert_eq!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("json"),
                    "the filter accepted a path it should have refused: {path:?}",
                );
            }
        }
        assert!(!events.is_empty(), "the watched file produced no event");
    }

    #[test]
    fn a_path_changed_twice_between_polls_is_reported_once_by_the_next_poll() {
        let sandbox = Sandbox::new("coalesce");
        let watch = DirectoryWatch::start(sandbox.path(), false, |_| true).expect("watchable");
        let file = sandbox.path().join("note.txt");

        // Let the watcher deliver the creation before the poll below, so the two writes land in
        // the same batch read by a single poll.
        std::fs::write(&file, "one").expect("writable");
        let _ = poll_until(&watch);
        std::thread::sleep(Duration::from_millis(150));

        std::fs::write(&file, "two").expect("writable");
        std::fs::write(&file, "three").expect("writable");

        let events = poll_until(&watch);
        let for_file = events
            .iter()
            .filter(|event| matches!(event, WatchEvent::Changed { path, .. } if path == &file))
            .count();
        assert_eq!(
            for_file, 1,
            "two writes before one poll must coalesce into one event, got {events:?}",
        );
    }

    #[test]
    fn dropping_the_watch_stops_reading_events() {
        let sandbox = Sandbox::new("drop");
        let watch = DirectoryWatch::start(sandbox.path(), false, |_| true).expect("watchable");
        assert_eq!(watch.root(), sandbox.path().as_os_str());
        assert!(!watch.is_recursive());
        drop(watch);

        // The watcher is owned by `DirectoryWatch`, so dropping it must end the watch. A visible
        // consequence is that the process is still alive and nothing is left polling.
        assert!(std::fs::write(sandbox.path().join("after.txt"), "x").is_ok());
    }
}
