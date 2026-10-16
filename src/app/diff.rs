// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Two-sided and inline diffs, directory comparison and a read-only revision view (task T08.07).
//!
//! The acceptance line is *"a line-ending or encoding difference is not hidden as identical"*. That
//! is a statement about what the comparison is **allowed to overlook**, so the module is built so
//! that overlooking nothing is the default and overlooking something is always something a caller
//! asked for:
//!
//! * **A line keeps its ending.** [`split_lines`] cuts after each `\n`, so `"a\n"` and `"a\r\n"` are
//!   two different lines, not one line read twice. Two texts that differ only in their endings are
//!   therefore a [`Row::Change`], never a silent [`Row::Equal`].
//! * **The only things ignored are asked for.** [`Options`] carries `ignore_line_endings` and
//!   `ignore_trailing_whitespace`; both default to **off**. When one is on, the line keys lose the
//!   thing it names, and the report says so — ignoring is an explicit act with a record.
//! * **A changed pair is shown inside as well as between.** A [`Row::Change`] carries the character
//!   [`Segment`]s of its own two lines, so a `\r` that only differs in an ending is visible as an
//!   added character rather than a whole line that merely "changed".
//! * **A revision is read-only by construction.** [`Revision`] has no edit path that succeeds; an
//!   edit returns a refusal, so a read-only revision view cannot quietly become writable.
//!
//! **Boundaries.** The line alignment is a longest-common-subsequence table, which is `O(n*m)`; past
//! [`MAX_CELLS`] it degrades to "everything is one change" rather than allocating a table that would
//! not fit. The inline pass has its own [`MAX_INLINE_CELLS`]. Encoding is [T06.05](crate::project::text_file)'s
//! subject: a decoded BOM is a `\u{feff}` character here, so a BOM difference shows up as an ordinary
//! text difference instead of being normalised away.

#![allow(dead_code)]

/// Above this many table cells the line alignment degrades to one whole change.
pub const MAX_CELLS: usize = 4_000_000;
/// Above this many table cells the inline pass degrades to two whole segments.
pub const MAX_INLINE_CELLS: usize = 1_000_000;

/// What a comparison is allowed to overlook. Nothing, unless a caller says otherwise.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Options {
    /// Treat `\r\n` and `\n` as the same ending (off by default, so an ending shows as a difference).
    pub ignore_line_endings: bool,
    /// Ignore trailing spaces and tabs before the ending (off by default).
    pub ignore_trailing_whitespace: bool,
}

impl Options {
    /// The strict default: nothing is overlooked.
    pub const fn strict() -> Self {
        Self {
            ignore_line_endings: false,
            ignore_trailing_whitespace: false,
        }
    }

    /// The key a line is compared by, under these options.
    fn key(&self, line: &str) -> String {
        let mut key = line.to_owned();
        if self.ignore_line_endings
            && let Some(stripped) = key.strip_suffix("\r\n")
        {
            key = format!("{stripped}\n");
        }
        if self.ignore_trailing_whitespace {
            let ending = if key.ends_with('\n') { "\n" } else { "" };
            let body = key.strip_suffix(ending).unwrap_or(&key);
            key = format!("{}{ending}", body.trim_end_matches([' ', '\t']));
        }
        key
    }
}

/// One segment of an inline comparison.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Segment {
    /// Characters on both sides.
    Same(String),
    /// Characters only on the left.
    Remove(String),
    /// Characters only on the right.
    Add(String),
}

impl Segment {
    /// The segment as one line.
    pub fn render(&self) -> String {
        match self {
            Self::Same(text) => format!("  = {text:?}"),
            Self::Remove(text) => format!("  - {text:?}"),
            Self::Add(text) => format!("  + {text:?}"),
        }
    }
}

/// One row of a two-sided diff.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Row {
    /// A line present on both sides.
    Equal {
        /// Its index on the left.
        left: usize,
        /// Its index on the right.
        right: usize,
        /// The line.
        text: String,
    },
    /// A line only on the left.
    Delete {
        /// Its index on the left.
        left: usize,
        /// The line.
        text: String,
    },
    /// A line only on the right.
    Insert {
        /// Its index on the right.
        right: usize,
        /// The line.
        text: String,
    },
    /// A line replaced by another, shown down to the characters that differ.
    Change {
        /// Its index on the left.
        left: usize,
        /// Its index on the right.
        right: usize,
        /// The line on the left.
        from: String,
        /// The line on the right.
        to: String,
        /// The character segments of the two lines.
        inline: Vec<Segment>,
    },
}

impl Row {
    /// The row in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Equal { text, .. } => format!("  {text:?}"),
            Self::Delete { text, .. } => format!("- {text:?}"),
            Self::Insert { text, .. } => format!("+ {text:?}"),
            Self::Change { from, to, .. } => format!("~ {from:?} -> {to:?}"),
        }
    }

    /// Whether the row is a difference.
    pub fn is_change(&self) -> bool {
        !matches!(self, Self::Equal { .. })
    }
}

/// Splits a text into lines, each keeping its ending. See the module docs.
pub fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(&text[start..=index]);
            start = index + 1;
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// Compares two texts line by line, and every changed pair down to its characters.
pub fn diff(left: &str, right: &str, options: Options) -> Vec<Row> {
    let left_lines = split_lines(left);
    let right_lines = split_lines(right);
    let left_keys: Vec<String> = left_lines.iter().map(|line| options.key(line)).collect();
    let right_keys: Vec<String> = right_lines.iter().map(|line| options.key(line)).collect();
    let steps = align(&left_keys, &right_keys);
    rows_from_steps(&steps, &left_lines, &right_lines)
}

/// The number of changed, inserted or deleted rows.
pub fn changed_rows(rows: &[Row]) -> usize {
    rows.iter().filter(|row| row.is_change()).count()
}

/// Whether two texts are the same under these options.
pub fn identical(left: &str, right: &str, options: Options) -> bool {
    let left_lines = split_lines(left);
    let right_lines = split_lines(right);
    left_lines.len() == right_lines.len()
        && left_lines
            .iter()
            .zip(&right_lines)
            .all(|(a, b)| options.key(a) == options.key(b))
}

/// A file in a directory comparison.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TreeEntry {
    /// The path, relative to the tree's root.
    pub path: String,
    /// The file's text.
    pub text: String,
}

impl TreeEntry {
    /// An entry of a path and its text.
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            text: text.into(),
        }
    }
}

/// How one path differs between two trees.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    /// The same content.
    Identical,
    /// Present on both sides, with different content.
    Changed,
    /// Only on the right.
    Added,
    /// Only on the left.
    Removed,
}

impl FileKind {
    /// The kind's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Identical => "identical",
            Self::Changed => "changed",
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }
}

/// One path's outcome in a directory comparison.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileDiff {
    /// The path.
    pub path: String,
    /// How it differs.
    pub kind: FileKind,
    /// The two-sided diff, for a changed or one-sided file.
    pub rows: Vec<Row>,
}

impl FileDiff {
    /// The file diff in one line.
    pub fn render(&self) -> String {
        if self.rows.is_empty() {
            format!("  {}: {}", self.path, self.kind.name())
        } else {
            format!(
                "  {}: {}, {} changed row(s)",
                self.path,
                self.kind.name(),
                changed_rows(&self.rows)
            )
        }
    }
}

/// Compares two directory listings by path.
pub fn compare_trees(left: &[TreeEntry], right: &[TreeEntry], options: Options) -> Vec<FileDiff> {
    let mut paths: Vec<&str> = left
        .iter()
        .map(|entry| entry.path.as_str())
        .chain(right.iter().map(|entry| entry.path.as_str()))
        .collect();
    paths.sort_unstable();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let from = left.iter().find(|entry| entry.path == path);
            let to = right.iter().find(|entry| entry.path == path);
            match (from, to) {
                (Some(from), Some(to)) => {
                    let rows = diff(&from.text, &to.text, options);
                    let kind = if changed_rows(&rows) == 0 {
                        FileKind::Identical
                    } else {
                        FileKind::Changed
                    };
                    FileDiff {
                        path: path.to_owned(),
                        kind,
                        rows,
                    }
                }
                (Some(from), None) => FileDiff {
                    path: path.to_owned(),
                    kind: FileKind::Removed,
                    rows: diff(&from.text, "", options),
                },
                (None, Some(to)) => FileDiff {
                    path: path.to_owned(),
                    kind: FileKind::Added,
                    rows: diff("", &to.text, options),
                },
                (None, None) => unreachable!("the path came from one of the two trees"),
            }
        })
        .collect()
}

/// A read-only revision: one side of a comparison, as it was, which cannot be edited.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Revision {
    name: String,
    text: String,
}

impl Revision {
    /// A revision with a name and its text. It is always read-only.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }

    /// The revision's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The revision's text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the revision can be edited. It cannot.
    pub const fn is_read_only(&self) -> bool {
        true
    }

    /// Refuses every edit, so a read-only view stays read-only.
    pub fn edit(&self, at: usize, remove: usize) -> Result<(), String> {
        Err(format!(
            "the revision {} is read-only; an edit at {at} removing {remove} byte(s) is refused",
            self.name
        ))
    }

    /// The revision in one line.
    pub fn render(&self) -> String {
        format!(
            "revision {}: {} byte(s), read-only",
            self.name,
            self.text.len()
        )
    }
}

/// A step of the line alignment: the left line kept, dropped, or a right line added.
type Step = (Option<usize>, Option<usize>);

/// The longest common subsequence alignment of two key sequences.
fn align(left: &[String], right: &[String]) -> Vec<Step> {
    let (n, m) = (left.len(), right.len());
    if n.saturating_mul(m) > MAX_CELLS {
        let mut steps = Vec::with_capacity(n + m);
        steps.extend(
            left.iter()
                .enumerate()
                .map(|(index, _)| (Some(index), None)),
        );
        steps.extend(
            right
                .iter()
                .enumerate()
                .map(|(index, _)| (None, Some(index))),
        );
        return steps;
    }
    // dp[i][j] is the length of the alignment of left[i..] and right[j..].
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if left[i] == right[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut steps = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if left[i] == right[j] {
            steps.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            steps.push((Some(i), None));
            i += 1;
        } else {
            steps.push((None, Some(j)));
            j += 1;
        }
    }
    while i < n {
        steps.push((Some(i), None));
        i += 1;
    }
    while j < m {
        steps.push((None, Some(j)));
        j += 1;
    }
    steps
}

/// Coalesces an alignment into rows, pairing a run of deletes with a run of inserts as changes.
fn rows_from_steps(steps: &[Step], left: &[&str], right: &[&str]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < steps.len() {
        if let (Some(i), Some(j)) = steps[index] {
            rows.push(Row::Equal {
                left: i,
                right: j,
                text: left[i].to_owned(),
            });
            index += 1;
            continue;
        }
        let mut lefts = Vec::new();
        let mut rights = Vec::new();
        while index < steps.len() {
            match steps[index] {
                (Some(i), None) => {
                    lefts.push(i);
                    index += 1;
                }
                (None, Some(j)) => {
                    rights.push(j);
                    index += 1;
                }
                _ => break,
            }
        }
        let paired = lefts.len().min(rights.len());
        for k in 0..paired {
            let from = left[lefts[k]];
            let to = right[rights[k]];
            rows.push(Row::Change {
                left: lefts[k],
                right: rights[k],
                from: from.to_owned(),
                to: to.to_owned(),
                inline: inline(from, to),
            });
        }
        for &i in &lefts[paired..] {
            rows.push(Row::Delete {
                left: i,
                text: left[i].to_owned(),
            });
        }
        for &j in &rights[paired..] {
            rows.push(Row::Insert {
                right: j,
                text: right[j].to_owned(),
            });
        }
    }
    rows
}

/// The character segments of two lines that differ.
pub fn inline(from: &str, to: &str) -> Vec<Segment> {
    let left: Vec<char> = from.chars().collect();
    let right: Vec<char> = to.chars().collect();
    let (n, m) = (left.len(), right.len());
    if n.saturating_mul(m) > MAX_INLINE_CELLS {
        return vec![
            Segment::Remove(from.to_owned()),
            Segment::Add(to.to_owned()),
        ];
    }
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if left[i] == right[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut ops: Vec<(char, u8)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if left[i] == right[j] {
            ops.push((left[i], 0));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push((left[i], 1));
            i += 1;
        } else {
            ops.push((right[j], 2));
            j += 1;
        }
    }
    while i < n {
        ops.push((left[i], 1));
        i += 1;
    }
    while j < m {
        ops.push((right[j], 2));
        j += 1;
    }
    // Group runs of the same kind into segments.
    let mut segments: Vec<Segment> = Vec::new();
    for (character, kind) in ops {
        let appended = match segments.last_mut() {
            Some(Segment::Same(text)) if kind == 0 => {
                text.push(character);
                true
            }
            Some(Segment::Remove(text)) if kind == 1 => {
                text.push(character);
                true
            }
            Some(Segment::Add(text)) if kind == 2 => {
                text.push(character);
                true
            }
            _ => false,
        };
        if !appended {
            segments.push(match kind {
                0 => Segment::Same(character.to_string()),
                1 => Segment::Remove(character.to_string()),
                _ => Segment::Add(character.to_string()),
            });
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::{
        FileKind, Options, Revision, Row, Segment, TreeEntry, compare_trees, diff, identical,
        inline, split_lines,
    };

    #[test]
    fn a_line_ending_difference_is_not_hidden_as_identical() {
        let left = "a\nb\n";
        let right = "a\r\nb\n";
        // Strict: the first lines differ, and the difference is the carriage return itself.
        let rows = diff(left, right, Options::strict());
        assert_eq!(rows.len(), 2);
        match &rows[0] {
            Row::Change { inline, .. } => assert!(
                inline.contains(&Segment::Add("\r".to_owned())),
                "the carriage return is not shown: {inline:?}"
            ),
            other => panic!("expected a change, got {other:?}"),
        }
        assert!(!identical(left, right, Options::strict()));
        // Asked to ignore endings: now they are the same, and that is a recorded choice.
        let ignoring = Options {
            ignore_line_endings: true,
            ignore_trailing_whitespace: false,
        };
        assert!(identical(left, right, ignoring));
        assert_eq!(diff(left, right, ignoring).len(), 2);
    }

    #[test]
    fn a_bom_difference_is_a_difference() {
        let left = "a\n";
        let right = "\u{feff}a\n";
        let rows = diff(left, right, Options::strict());
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_change());
    }

    #[test]
    fn a_single_changed_character_shows_inside_the_line() {
        let rows = diff("let x = 1;\n", "let x = 2;\n", Options::strict());
        match &rows[0] {
            Row::Change { inline, .. } => {
                assert_eq!(inline.first(), Some(&Segment::Same("let x = ".to_owned())));
                assert!(
                    inline.contains(&Segment::Remove("1".to_owned())),
                    "the removed character is not shown: {inline:?}"
                );
                assert!(
                    inline.contains(&Segment::Add("2".to_owned())),
                    "the added character is not shown: {inline:?}"
                );
            }
            other => panic!("expected a change, got {other:?}"),
        }
    }

    #[test]
    fn an_inserted_and_a_deleted_line_are_reported_apart() {
        let rows = diff("a\nb\nc\n", "a\nc\nd\n", Options::strict());
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, Row::Delete { .. }))
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, Row::Insert { .. }))
                .count(),
            1
        );
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, Row::Equal { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn split_lines_keeps_the_ending() {
        assert_eq!(split_lines("a\nb"), vec!["a\n", "b"]);
        assert_eq!(split_lines("a\r\nb\n"), vec!["a\r\n", "b\n"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
    }

    #[test]
    fn a_trailing_whitespace_difference_is_only_ignored_when_asked() {
        let left = "a \n";
        let right = "a\n";
        assert!(!identical(left, right, Options::strict()));
        let ignoring = Options {
            ignore_line_endings: false,
            ignore_trailing_whitespace: true,
        };
        assert!(identical(left, right, ignoring));
    }

    #[test]
    fn a_directory_comparison_names_every_kind() {
        let left = vec![
            TreeEntry::new("same.txt", "x\n"),
            TreeEntry::new("changed.txt", "a\n"),
            TreeEntry::new("gone.txt", "g\n"),
        ];
        let right = vec![
            TreeEntry::new("same.txt", "x\n"),
            TreeEntry::new("changed.txt", "b\n"),
            TreeEntry::new("new.txt", "n\n"),
        ];
        let diffs = compare_trees(&left, &right, Options::strict());
        let by_path: Vec<(&str, FileKind)> = diffs
            .iter()
            .map(|diff| (diff.path.as_str(), diff.kind))
            .collect();
        assert_eq!(
            by_path,
            vec![
                ("changed.txt", FileKind::Changed),
                ("gone.txt", FileKind::Removed),
                ("new.txt", FileKind::Added),
                ("same.txt", FileKind::Identical),
            ]
        );
    }

    #[test]
    fn an_ending_only_change_is_changed_not_identical_in_a_tree() {
        let left = vec![TreeEntry::new("a.txt", "x\n")];
        let right = vec![TreeEntry::new("a.txt", "x\r\n")];
        let diffs = compare_trees(&left, &right, Options::strict());
        assert_eq!(diffs[0].kind, FileKind::Changed);
    }

    #[test]
    fn a_revision_refuses_every_edit() {
        let revision = Revision::new("HEAD", "one\ntwo\n");
        assert!(revision.is_read_only());
        assert_eq!(revision.text(), "one\ntwo\n");
        let refusal = revision.edit(4, 3).expect_err("the edit is refused");
        assert!(refusal.contains("read-only"));
        assert!(refusal.contains("HEAD"));
    }

    #[test]
    fn an_inline_diff_of_equal_lines_has_one_same_segment() {
        assert_eq!(inline("abc", "abc"), vec![Segment::Same("abc".to_owned())]);
    }

    #[test]
    fn a_very_large_comparison_does_not_build_a_table_that_cannot_fit() {
        // Two wide inputs put the line table past its ceiling; nothing is allocated for the table.
        let left = "a\n".repeat(3000);
        let right = "b\n".repeat(3000);
        let rows = diff(&left, &right, Options::strict());
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.is_change()));
    }
}
