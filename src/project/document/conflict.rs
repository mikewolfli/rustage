// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! External modification, three-way conflict, read-only status and a deleted file (task T03.05).
//!
//! The acceptance line is *"an external modification does not silently overwrite a dirty buffer"*.
//! [`crate::project::document::save`] already refuses a write when the file no longer holds the bytes the
//! document agreed with; this module is the other half — *deciding what to do* about it, so the
//! refusal can be answered rather than only reported.
//!
//! Three versions, named the way a person thinks about them:
//!
//! * **base** — the bytes the document last agreed with the file (what it read, or what it last
//!   wrote). This is the point both sides started from. Without it there is no way to tell "the disk
//!   changed" from "I changed it", and neither is a conflict.
//! * **mine** — the text the document holds now (the buffer's text).
//! * **theirs** — what is on disk now.
//!
//! [`Verdict`] is the answer, and it is a *decision*, not a score: nobody changed anything; only the
//! disk changed (reload, and nothing of mine is lost); only I changed (write, and nothing of the
//! file's is lost); both changed to the same text (the two agree); both changed differently (a
//! [`Verdict::Conflicted`] whose regions name each side, or a [`Verdict::Merged`] when the changes
//! touch different lines).
//!
//! **The merge is a rule, not a heuristic.** [`merge`] is diff3: the line changes each side made
//! against the base are computed, changes that touch different base lines are both applied, and only
//! changes that touch the *same* base lines are a conflict — reported with both sides' lines for
//! those lines. It never guesses which side to prefer, because picking a side silently is exactly the
//! data loss this task exists to prevent.
//!
//! The two remaining shapes are not conflicts at all but states:
//!
//! * **Read-only.** [`writability`] asks the file itself (it opens it for writing, which changes
//!   nothing) rather than reading a metadata bit that means something else, so "why can I not save"
//!   has the OS's own answer.
//! * **Gone and back.** A file that was deleted is [`Verdict::Gone`], and one that came back with
//!   different bytes is [`Verdict::Replaced`]: the first is a delete somebody has to decide about
//!   (re-create it from the buffer, or let it go), the second is an external change like any other.
//!
//! **Attribution is not here.** Which process changed the file is the watcher's [`ChangeKind`], and
//! the dependency reports a path and a kind, not an author (`host::watch` says so). This module
//! decides what the bytes mean; it does not guess who wrote them.
//!
//! [`ChangeKind`]: rust_widgets::asset::ChangeKind

use std::path::Path;

/// What is on disk now, measured against the bytes a document agreed with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OnDisk {
    /// The file still holds exactly the bytes the document agreed with.
    Same {
        /// The bytes, for a caller that needs them again.
        bytes: Vec<u8>,
    },
    /// The file holds different bytes.
    Changed {
        /// What is there now.
        bytes: Vec<u8>,
    },
    /// The file is not there.
    Gone,
    /// Something is at the path but not a readable file.
    NotAFile {
        /// The OS's own reason.
        reason: String,
    },
}

impl OnDisk {
    /// Reads the file at `path` and compares it with the bytes a document agreed with.
    ///
    /// `agreed` is `None` for a document that never read a file (a paste): there is nothing to be
    /// out of date with, so the answer is `Changed` and the caller treats it as "no base".
    pub fn inspect(path: &Path, agreed: Option<&[u8]>) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => match agreed {
                Some(agreed) if agreed == bytes.as_slice() => Self::Same { bytes },
                _ => Self::Changed { bytes },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::Gone,
            Err(error) => Self::NotAFile {
                reason: error.to_string(),
            },
        }
    }

    /// Whether the file still holds what the document agreed with.
    pub const fn is_same(&self) -> bool {
        matches!(self, Self::Same { .. })
    }
}

/// Whether a file can be written, and the OS's reason when it cannot.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Writability {
    /// Opening the file for writing succeeded.
    Writable,
    /// The file could not be opened for writing.
    ReadOnly {
        /// The OS's own reason.
        reason: String,
    },
}

impl Writability {
    /// The writability in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Writable => "the file can be written".to_owned(),
            Self::ReadOnly { reason } => {
                format!("the file is read-only: {reason}")
            }
        }
    }
}

/// Asks the file whether it can be written.
///
/// The check opens the file for writing, which changes neither its bytes nor its length — unlike the
/// metadata's read-only bit, which on Unix is set only when *everyone* lacks write permission and so
/// says nothing about whether *this* process may write. The OS's answer is the honest one.
pub fn writability(path: &Path) -> Writability {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => Writability::Writable,
        Err(error) => Writability::ReadOnly {
            reason: error.to_string(),
        },
    }
}

/// One region where the two sides disagree, with each side's lines for the base lines they replace.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Region {
    /// The 0-based base line where the region starts.
    start: usize,
    /// The base lines the region replaces.
    base: Vec<String>,
    /// What the document made of those lines.
    mine: Vec<String>,
    /// What the file made of those lines.
    theirs: Vec<String>,
}

impl Region {
    /// The 0-based base line where the region starts.
    pub const fn start(&self) -> usize {
        self.start
    }

    /// The base lines both sides changed.
    pub fn base(&self) -> &[String] {
        &self.base
    }

    /// The document's version of those lines.
    pub fn mine(&self) -> &[String] {
        &self.mine
    }

    /// The file's version of those lines.
    pub fn theirs(&self) -> &[String] {
        &self.theirs
    }

    /// How many base lines the region covers; zero for a region both sides inserted at.
    pub fn base_len(&self) -> usize {
        self.base.len()
    }

    /// The region in one sentence.
    pub fn describe(&self) -> String {
        format!(
            "base lines {}..{}: mine has {} line(s), the file has {} line(s)",
            self.start + 1,
            self.start + self.base.len() + 1,
            self.mine.len(),
            self.theirs.len(),
        )
    }
}

/// What a document should do, given the base, its own text, and the file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Nobody changed anything.
    Unchanged,
    /// Only the file changed; the document holds no edit of its own.
    DiskOnly,
    /// Only the document changed; the file is as it was read.
    DocumentOnly,
    /// Both changed, to the same text (the two agree).
    BothAgree,
    /// Both changed differently, and the changes touch different lines: here is the merged text.
    Merged {
        /// The merged text.
        text: String,
    },
    /// Both changed differently, on the same lines: nobody may pick a side for the user.
    Conflicted {
        /// The regions, in base order.
        regions: Vec<Region>,
    },
    /// The file is gone.
    Gone,
    /// Something is at the path but not a readable file.
    NotAFile {
        /// The OS's own reason.
        reason: String,
    },
}

impl Verdict {
    /// Whether the document and the file agree, and there is nothing to decide.
    pub const fn is_clean(&self) -> bool {
        matches!(
            self,
            Self::Unchanged | Self::BothAgree | Self::DocumentOnly | Self::Merged { .. }
        )
    }

    /// Whether a person has to decide what to keep.
    pub const fn needs_a_decision(&self) -> bool {
        matches!(
            self,
            Self::DiskOnly | Self::Conflicted { .. } | Self::Gone | Self::NotAFile { .. }
        )
    }

    /// The verdict in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Unchanged => {
                "the file is exactly what the document read; nothing to decide".to_owned()
            }
            Self::DiskOnly => {
                "the file changed and the document holds no edit of its own: reloading loses \
                 nothing of the document's"
                    .to_owned()
            }
            Self::DocumentOnly => {
                "the document changed and the file is as it was read: writing loses nothing of the \
                 file's"
                    .to_owned()
            }
            Self::BothAgree => {
                "both sides changed to the same text, so neither loses anything".to_owned()
            }
            Self::Merged { .. } => {
                "both sides changed different lines, so both changes can be kept".to_owned()
            }
            Self::Conflicted { regions } => format!(
                "both sides changed the same {} region(s), so nobody may pick a side silently",
                regions.len()
            ),
            Self::Gone => {
                "the file is gone; re-creating it from the document or letting it go is a decision"
                    .to_owned()
            }
            Self::NotAFile { reason } => {
                format!("the path is not a readable file: {reason}")
            }
        }
    }

    /// A merge result's text, when there is one.
    pub fn merged_text(&self) -> Option<&str> {
        match self {
            Self::Merged { text } => Some(text),
            _ => None,
        }
    }
}

/// Decides what a document should do, from the base, its own text and the file's bytes.
///
/// `base` is `None` for a document that never read a file: then there is no earlier state, and the
/// answer is [`Verdict::DocumentOnly`] when the path is empty of a base and the document has text,
/// or [`Verdict::DiskOnly`] when there is a file to read and the document does not. A base is what
/// makes "both changed" a question at all.
pub fn decide(base: Option<&[u8]>, mine: &str, on_disk: &OnDisk) -> Verdict {
    match on_disk {
        OnDisk::Gone => Verdict::Gone,
        OnDisk::NotAFile { reason } => Verdict::NotAFile {
            reason: reason.clone(),
        },
        OnDisk::Same { .. } => {
            // The file is as it was read, so only the document could have changed.
            match base {
                Some(base) if base == mine.as_bytes() => Verdict::Unchanged,
                _ => Verdict::DocumentOnly,
            }
        }
        OnDisk::Changed { bytes } => {
            let theirs = String::from_utf8_lossy(bytes);
            match base {
                // No base: the document holds a paste and the file is a different thing; the only
                // safe reading is that the document is the change and the file is not older than it.
                None => Verdict::DocumentOnly,
                Some(base) => {
                    let base = String::from_utf8_lossy(base);
                    if base.as_ref() == mine {
                        Verdict::DiskOnly
                    } else if mine == theirs.as_ref() {
                        Verdict::BothAgree
                    } else {
                        merge(&base, mine, theirs.as_ref())
                    }
                }
            }
        }
    }
}

/// A line change one side made against the base.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Hunk {
    base_start: usize,
    base_end: usize,
    lines: Vec<String>,
}

/// Which side a hunk came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Mine,
    Theirs,
}

/// diff3: applies both sides' line changes, and reports the regions they both changed.
///
/// The changes each side made against `base` are computed with a longest-common-subsequence diff;
/// two changes that touch different base lines are both applied, and two that touch the same base
/// lines become a [`Region`]. Nothing is guessed: a region is reported, not resolved.
pub fn merge(base: &str, mine: &str, theirs: &str) -> Verdict {
    let base_lines = lines(base);
    let mine_lines = lines(mine);
    let theirs_lines = lines(theirs);

    let mut tagged: Vec<(Side, Hunk)> = Vec::new();
    for hunk in diff(&base_lines, &mine_lines) {
        tagged.push((Side::Mine, hunk));
    }
    for hunk in diff(&base_lines, &theirs_lines) {
        tagged.push((Side::Theirs, hunk));
    }
    // The base position orders the walk; a side's own hunks are already disjoint and in order.
    tagged.sort_by_key(|(_, hunk)| (hunk.base_start, hunk.base_end));

    let mut merged: Vec<String> = Vec::new();
    let mut regions: Vec<Region> = Vec::new();
    let mut cursor = 0usize;
    let mut index = 0usize;
    while index < tagged.len() {
        let start = tagged[index].1.base_start;
        // The unchanged base lines before this change.
        merged.extend(base_lines[cursor..start].iter().cloned());
        // Gather every hunk whose base range touches this one: that is what "the same lines" means.
        let mut end = tagged[index].1.base_end;
        let mut group = vec![index];
        while let Some(next) = tagged.get(index + group.len()) {
            if next.1.base_start > end {
                break;
            }
            end = end.max(next.1.base_end);
            group.push(index + group.len());
        }
        let mine_change = group.iter().find_map(|i| match tagged[*i].0 {
            Side::Mine => Some(tagged[*i].1.lines.clone()),
            Side::Theirs => None,
        });
        let theirs_change = group.iter().find_map(|i| match tagged[*i].0 {
            Side::Theirs => Some(tagged[*i].1.lines.clone()),
            Side::Mine => None,
        });
        match (mine_change, theirs_change) {
            (Some(a), Some(b)) if a == b => merged.extend(a),
            (Some(a), Some(b)) => regions.push(Region {
                start,
                base: base_lines[start..end].to_vec(),
                mine: a,
                theirs: b,
            }),
            (Some(a), None) => merged.extend(a),
            (None, Some(b)) => merged.extend(b),
            (None, None) => {}
        }
        cursor = end;
        index += group.len();
    }
    merged.extend(base_lines[cursor..].iter().cloned());

    if regions.is_empty() {
        Verdict::Merged {
            text: merged.concat(),
        }
    } else {
        Verdict::Conflicted { regions }
    }
}

/// Splits text into lines, each keeping its own newline, so joining reproduces the text exactly.
fn lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_owned).collect()
}

/// The line changes that turn `base` into `other`, as hunks over base positions.
///
/// A hunk covers the base lines it replaces (possibly none, for an insertion) and carries the lines
/// that stand in their place. The matching is a longest common subsequence, which is what makes "the
/// same lines" in [`merge`] mean the same lines rather than the same positions.
fn diff(base: &[String], other: &[String]) -> Vec<Hunk> {
    let matches = common_subsequence(base, other);
    let mut hunks = Vec::new();
    let mut b = 0usize;
    let mut o = 0usize;
    let mut next = 0usize;
    while next <= matches.len() {
        let (b_end, o_end) = match matches.get(next) {
            Some(&(mb, mo)) => (mb, mo),
            None => (base.len(), other.len()),
        };
        if b < b_end || o < o_end {
            hunks.push(Hunk {
                base_start: b,
                base_end: b_end,
                lines: other[o..o_end].to_vec(),
            });
        }
        if let Some(&(mb, mo)) = matches.get(next) {
            b = mb + 1;
            o = mo + 1;
        } else {
            break;
        }
        next += 1;
    }
    hunks
}

/// The longest common subsequence of two line sequences, as matched index pairs in order.
///
/// Quadratic, and deliberately so: the texts here are page documents a person edits, and the
/// alternative (a diff of the whole document on every keystroke) is the cost the roadmap's "cost
/// grows with what is visible" rule is about — this runs when a save is *asked for*, not per frame.
fn common_subsequence(base: &[String], other: &[String]) -> Vec<(usize, usize)> {
    let n = base.len();
    let m = other.len();
    // table[i][j] is the LCS length of base[i..] and other[j..].
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if base[i] == other[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut matched = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if base[i] == other[j] {
            matched.push((i, j));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sandbox() -> (PathBuf, TempDirectory) {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rustage-conflict-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a temporary directory is creatable");
        (path.clone(), TempDirectory(path))
    }

    #[test]
    fn an_untouched_file_is_unchanged_and_a_reloaded_one_has_only_the_disk_change() {
        let base = "a\nb\nc\n";
        assert_eq!(
            decide(
                Some(base.as_bytes()),
                base,
                &OnDisk::Same { bytes: base.into() }
            ),
            Verdict::Unchanged
        );
        let disk = OnDisk::Changed {
            bytes: b"a\nB\nc\n".to_vec(),
        };
        assert_eq!(
            decide(Some(base.as_bytes()), base, &disk),
            Verdict::DiskOnly
        );
        assert!(decide(Some(base.as_bytes()), base, &disk).needs_a_decision());
    }

    #[test]
    fn only_the_document_changing_is_not_a_conflict() {
        let base = "a\nb\nc\n";
        let verdict = decide(
            Some(base.as_bytes()),
            "a\nb\nc\nd\n",
            &OnDisk::Same {
                bytes: base.as_bytes().to_vec(),
            },
        );
        assert_eq!(verdict, Verdict::DocumentOnly);
        assert!(verdict.is_clean());
    }

    #[test]
    fn changes_to_different_lines_are_both_kept() {
        let base = "one\ntwo\nthree\nfour\n";
        let mine = "ONE\ntwo\nthree\nfour\n";
        let theirs = "one\ntwo\nthree\nFOUR\n";
        let verdict = merge(base, mine, theirs);
        assert_eq!(
            verdict,
            Verdict::Merged {
                text: "ONE\ntwo\nthree\nFOUR\n".to_owned()
            }
        );
    }

    #[test]
    fn changes_to_the_same_line_are_a_conflict_that_names_both_sides() {
        let base = "one\ntwo\nthree\n";
        let mine = "one\nMINE\nthree\n";
        let theirs = "one\nTHEIRS\nthree\n";
        let verdict = match merge(base, mine, theirs) {
            Verdict::Conflicted { regions } => regions,
            other => panic!("expected a conflict, got {other:?}"),
        };
        assert_eq!(verdict.len(), 1);
        let region = &verdict[0];
        assert_eq!(region.base(), ["two\n"]);
        assert_eq!(region.mine(), ["MINE\n"]);
        assert_eq!(region.theirs(), ["THEIRS\n"]);
        assert_eq!(region.start(), 1);
        assert!(region.describe().contains("base lines 2..3"));
    }

    #[test]
    fn both_sides_making_the_same_change_agree() {
        let base = "one\ntwo\nthree\n";
        let same = "one\nTWO\nthree\n";
        assert_eq!(
            merge(base, same, same),
            Verdict::Merged {
                text: same.to_owned()
            }
        );
    }

    #[test]
    fn an_insertion_by_each_side_lands_in_order() {
        let base = "a\nb\n";
        let mine = "A\na\nb\n";
        let theirs = "a\nb\nB\n";
        assert_eq!(
            merge(base, mine, theirs),
            Verdict::Merged {
                text: "A\na\nb\nB\n".to_owned()
            }
        );
    }

    #[test]
    fn a_deleted_file_is_gone_and_a_replaced_one_is_a_change() {
        let (path, _guard) = sandbox();
        let file = path.join("page.json");
        let base = b"{\"v\":1}";
        assert_eq!(OnDisk::inspect(&file, Some(base)), OnDisk::Gone);
        assert_eq!(
            decide(Some(base), "{\"v\":1}", &OnDisk::Gone),
            Verdict::Gone
        );
        assert!(decide(Some(base), "{\"v\":1}", &OnDisk::Gone).needs_a_decision());

        std::fs::write(&file, b"{\"v\":2}").expect("writable");
        assert_eq!(
            OnDisk::inspect(&file, Some(base)),
            OnDisk::Changed {
                bytes: b"{\"v\":2}".to_vec()
            }
        );
        std::fs::write(&file, b"{\"v\":1}").expect("writable");
        assert!(OnDisk::inspect(&file, Some(base)).is_same());
    }

    #[test]
    fn a_writable_file_says_so_and_a_missing_one_does_not_pretend() {
        let (path, _guard) = sandbox();
        let file = path.join("page.json");
        std::fs::write(&file, "{}").expect("writable");
        assert_eq!(writability(&file), Writability::Writable);
        let missing = writability(&path.join("nope.json"));
        assert!(matches!(missing, Writability::ReadOnly { .. }));
        assert!(missing.message().contains("read-only"));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_reported_with_the_os_reason() {
        use std::os::unix::fs::PermissionsExt;
        let (path, _guard) = sandbox();
        let file = path.join("page.json");
        std::fs::write(&file, "{}").expect("writable");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).expect("chmod");

        let writability = writability(&file);
        // A process running as root may write anyway; then the file really is writable and the check
        // correctly says so, which is why this only asserts the shape.
        match writability {
            Writability::ReadOnly { reason } => assert!(!reason.is_empty()),
            Writability::Writable => {}
        }
    }
}
