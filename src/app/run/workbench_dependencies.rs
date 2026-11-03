// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The Cargo members and dependencies panel's rows (roadmap F02, `panel.dependencies`).
//!
//! The panel reads two things and claims nothing else: the lockfile beside `root`, and the
//! resolution this build *would* run. It does not start cargo, so it cannot claim an enabled
//! feature set, a dependency tree cargo has not printed, or a pending lockfile change that only a
//! resolution can detect.
//!
//! * the manifest and lockfile states come from [`manifest_of`] and [`Lockfile::read`]; an absent
//!   or unreadable lockfile is a row, and no package is invented to fill the panel;
//! * package rows are the `[[package]]` sections the lockfile actually names. [`Lockfile::read`]
//!   keeps only their count and [`crate::project::build::dependencies::Package`] is produced only
//!   from `cargo metadata`, which this panel must not run, so the names are read from the same file
//!   here; enabled features are not in a lockfile and the rows say so;
//! * the planned read-only resolution and the two updates a caller would have to authorize are
//!   [`Resolution`] values, rendered as command plus effects;
//! * the package list is capped so a large workspace cannot freeze the panel, and the number left
//!   out is stated (`omitted: N`) rather than silently dropped.

use std::path::Path;

use crate::project::build::dependencies::{LOCKFILE_FILE_NAME, Lockfile, Resolution, manifest_of};

/// The most package rows the panel builds; the rest are counted in an `omitted` row.
const PACKAGE_ROW_LIMIT: usize = 200;

/// The dependencies panel's rows for `root`.
pub(super) fn rows(root: &Path) -> Vec<String> {
    let mut rows = Vec::new();

    match manifest_of(root) {
        Ok(manifest) => rows.push(format!("manifest: {}", manifest.display())),
        Err(refusal) => rows.push(format!("manifest: {}", refusal.message())),
    }

    let lockfile = Lockfile::read(root);
    match &lockfile {
        Lockfile::Absent => rows.push(format!(
            "no {LOCKFILE_FILE_NAME} in {}: every version is still a choice, so no package is listed",
            root.display()
        )),
        present_or_unreadable => {
            rows.push(format!("lockfile: {}", present_or_unreadable.render()));
        }
    }

    if lockfile.is_present() {
        let path = root.join(LOCKFILE_FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => package_rows(&text, &mut rows),
            Err(error) => rows.push(format!(
                "{} could not be read for its packages: {error}",
                path.display()
            )),
        }
    }

    // A pending change is what `cargo metadata --locked` reports, and this panel does not run it:
    // saying that is more honest than reading the current lockfile as if it were the change state.
    rows.push(format!(
        "change state: not checked; a pending {LOCKFILE_FILE_NAME} change is what a resolution \
         reports, and this panel does not run cargo"
    ));

    rows.push(format!(
        "planned resolution: {}",
        Resolution::read_only(root).render()
    ));
    rows.push(format!(
        "authorized update, lockfile writable: {}",
        Resolution::may_write_the_lockfile(root).render()
    ));
    rows.push(format!(
        "authorized update, registry reachable: {}",
        Resolution::may_reach_the_registry(root).render()
    ));
    rows.push(
        "nothing is run by this panel: the effects above are what a caller would have to authorize"
            .to_owned(),
    );

    rows
}

/// The summary and package rows a lockfile's text supports.
fn package_rows(text: &str, rows: &mut Vec<String>) {
    let (packages, unnamed) = locked_packages(text);
    let with_source = packages
        .iter()
        .filter(|package| !package.source.is_empty())
        .count();
    let without_source = packages.len() - with_source;
    rows.push(format!(
        "packages: {} in {LOCKFILE_FILE_NAME}, {with_source} with a source (registry or git), \
         {without_source} with no source (local workspace members or path packages)",
        packages.len()
    ));
    if packages.is_empty() {
        rows.push(format!(
            "{LOCKFILE_FILE_NAME} names no package; nothing is invented to fill the panel"
        ));
    }
    if unnamed > 0 {
        rows.push(format!(
            "package entries not read: {unnamed} without a name"
        ));
    }
    for package in packages.iter().take(PACKAGE_ROW_LIMIT) {
        rows.push(package.row());
    }
    if packages.len() > PACKAGE_ROW_LIMIT {
        rows.push(format!(
            "omitted: {} package row(s) beyond the first {PACKAGE_ROW_LIMIT}",
            packages.len() - PACKAGE_ROW_LIMIT
        ));
    }
}

/// One `[[package]]` section, as the lockfile writes it.
#[derive(Default)]
struct Locked {
    name: String,
    version: String,
    source: String,
}

impl Locked {
    fn is_named(&self) -> bool {
        !self.name.is_empty()
    }

    fn row(&self) -> String {
        let version = if self.version.is_empty() {
            "(no version)"
        } else {
            self.version.as_str()
        };
        // A lockfile records no enabled features, and a missing source is not proof of membership:
        // a path dependency outside the workspace has no source either.
        let source = if self.source.is_empty() {
            "none (local or workspace member)"
        } else {
            self.source.as_str()
        };
        format!(
            "package: {} {version} · source: {source} · features: not recorded in \
             {LOCKFILE_FILE_NAME}",
            self.name
        )
    }
}

/// The packages a lockfile names, and how many `[[package]]` sections had no name.
fn locked_packages(text: &str) -> (Vec<Locked>, usize) {
    let mut packages = Vec::new();
    let mut unnamed = 0;
    let mut current: Option<Locked> = None;
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            flush(&mut current, &mut packages, &mut unnamed);
            current = Some(Locked::default());
            continue;
        }
        if line.starts_with('[') {
            flush(&mut current, &mut packages, &mut unnamed);
            continue;
        }
        let Some(package) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquoted(value.trim());
        match key.trim() {
            "name" => package.name = value,
            "version" => package.version = value,
            "source" => package.source = value,
            _ => {}
        }
    }
    flush(&mut current, &mut packages, &mut unnamed);
    (packages, unnamed)
}

fn flush(current: &mut Option<Locked>, packages: &mut Vec<Locked>, unnamed: &mut usize) {
    let Some(package) = current.take() else {
        return;
    };
    if package.is_named() {
        packages.push(package);
    } else {
        *unnamed += 1;
    }
}

/// The value of a lockfile `key = "value"` line, without its surrounding quotes.
fn unquoted(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::rows;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new(name: &str) -> Self {
            // Mirror the dependency service's naming: the clock alone can repeat inside one
            // process, and two tests sharing (then deleting) one directory would corrupt each other.
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustage-workbench-dependencies-{name}-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("a temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_directory_without_a_lockfile_says_so() {
        let directory = TempDirectory::new("no-lockfile");
        let rows = rows(directory.path());

        assert!(!rows.is_empty(), "the panel still answers with rows");
        assert!(
            rows.iter()
                .any(|row| row.contains("no Cargo.lock") && row.contains("no package")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_lockfile_with_a_package_reports_it() {
        let directory = TempDirectory::new("one-package");
        std::fs::write(
            directory.path().join("Cargo.lock"),
            concat!(
                "# This file is automatically @generated by Cargo.\n",
                "version = 3\n",
                "\n",
                "[[package]]\n",
                "name = \"example-crate\"\n",
                "version = \"0.1.0\"\n",
                "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
                "checksum = \"0000000000000000000000000000000000000000000000000000000000000000\"\n",
            ),
        )
        .expect("a Cargo.lock");

        let rows = rows(directory.path());

        assert!(
            rows.iter().any(|row| row.contains("example-crate")),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.contains("0.1.0")), "{rows:?}");
        assert!(
            rows.iter().any(|row| row.contains("registry+https://")),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("Cargo.lock v3")),
            "{rows:?}"
        );
    }
}
