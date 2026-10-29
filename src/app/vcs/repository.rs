// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! An existing Git repository, reached through the `git` program (task F20/T20.01–T20.05/T20.09).
//!
//! The designer does not implement a version system of its own: every operation is the installed
//! `git` command, and every answer is what that command printed. The rules this module keeps:
//!
//! * **A missing tool and a non-repository are different answers.** [`Discover::GitMissing`] and
//!   [`Discover::NotRepository`] let a caller keep designing and say which fact is missing.
//! * **Status is read fresh.** [`Repository::status`] runs `git status` on every call, so an
//!   external change cannot be mistaken for a cached clean tree.
//! * **Dirty means stop unless the caller says how.** Creating a branch and stashing require a
//!   clean worktree, and a plain switch refuses one too; [`Repository::switch_branch_stashing`] is
//!   the explicit alternative that sets the work aside, names the stash, and restores it when the
//!   switch fails. No path is dropped silently.
//! * **Destructive commands are bounded.** Merge/rebase/cherry-pick report their conflict and name
//!   the matching abort; nothing here pushes, force-updates a ref, deletes a worktree, or rewrites
//!   public history.
//! * **A conflict is three typed sides, not markers in a file.** [`Repository::conflicts`] asks
//!   `git ls-files -u` which stages exist and reads each with `git show :1:path`, `:2:path` and
//!   `:3:path`. The caller gets base/ours/theirs as values, so nothing has to parse conflict markers
//!   and nothing is merged on the caller's behalf.
//! * **Design and text conflicts resolve differently.** JSON/CSS (the design layer) need an explicit
//!   resolved document and are never auto-merged; text conflicts keep all three sides and offer a
//!   line/hunk selection. A text resolution of a generated file is refused when it would cross a
//!   hand-written boundary, and no resolution is reported as complete until the caller's
//!   regeneration step has passed.
//! * **A commit is previewed before it happens.** [`Repository::commit_preview`] reads only the
//!   index, reports what stays out, names an open buffer with unsaved changes, and scans the staged
//!   text for secrets ([`super::secrets`]). Nothing here stages, writes or commits to produce it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::app::vcs::secrets::{self, Secret};

/// Why a directory is not a repository this module can drive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Discover {
    /// `git` could not be started on this machine.
    GitMissing(String),
    /// `git` ran and said the path is not inside a repository.
    NotRepository(String),
}

impl Discover {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::GitMissing(reason) => format!("git is not available: {reason}"),
            Self::NotRepository(reason) => format!("not a git repository: {reason}"),
        }
    }
}

/// One `git status --porcelain` row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The index column (`M`, `A`, `D`, `R`, `?`, …).
    pub index: char,
    /// The worktree column.
    pub worktree: char,
    /// The project-relative path.
    pub path: String,
}

impl Entry {
    /// The row in one line.
    pub fn render(&self) -> String {
        format!("{}{} {}", self.index, self.worktree, self.path)
    }

    /// Whether this row is untracked.
    pub fn is_untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }
}

/// A repository's branch and changed paths at one point in time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Status {
    /// The current branch, or `HEAD` when detached.
    pub branch: String,
    /// Changed and untracked paths.
    pub entries: Vec<Entry>,
    /// Paths matched by ignore rules (`!!`), named so an unexpected ignore is diagnosable.
    pub ignored: Vec<String>,
}

impl Status {
    /// Whether there is no changed or untracked path.
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    /// The status in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "branch {}: {} changed path(s), {} ignored",
            self.branch,
            self.entries.len(),
            self.ignored.len()
        )];
        for entry in &self.entries {
            lines.push(format!(
                "  {}{}",
                entry.render(),
                if entry.is_untracked() {
                    " (untracked)"
                } else {
                    ""
                }
            ));
        }
        for path in &self.ignored {
            lines.push(format!("  !! {path}"));
        }
        lines
    }
}

/// Which side of a three-way conflict an index stage holds.
///
/// The numbers are Git's own: `1` is the merge base, `2` is ours (the current branch) and `3` is
/// theirs (the branch being merged in).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictStage {
    /// The merge base (`:1:path`).
    Base,
    /// The current branch's side (`:2:path`).
    Ours,
    /// The side being merged in (`:3:path`).
    Theirs,
}

impl ConflictStage {
    /// Every stage, in Git's order.
    pub const ALL: [Self; 3] = [Self::Base, Self::Ours, Self::Theirs];

    /// The stage number Git's `:N:path` spelling uses.
    pub const fn number(self) -> u8 {
        match self {
            Self::Base => 1,
            Self::Ours => 2,
            Self::Theirs => 3,
        }
    }

    /// The stage's name, as a person reads it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Ours => "ours",
            Self::Theirs => "theirs",
        }
    }

    /// Reads the number `git ls-files -u` printed.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "1" => Some(Self::Base),
            "2" => Some(Self::Ours),
            "3" => Some(Self::Theirs),
            _ => None,
        }
    }
}

/// How a conflicted path is resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictKind {
    /// A JSON design/project document: needs an explicit resolved document.
    Json,
    /// A CSS document: needs an explicit resolved document.
    Css,
    /// Ordinary text (including Rust): line/hunk selection, with generated-zone protection.
    Text,
}

impl ConflictKind {
    /// The kind's name, as a person reads it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Css => "css",
            Self::Text => "text",
        }
    }
}

/// The three sides of one conflicted path.
///
/// A side is absent when Git holds no stage for it: an add/add conflict has no base, and a
/// delete/modify conflict has no `ours` or `theirs` text. `None` says that rather than pretending an
/// empty file is the side.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Conflict {
    /// The project-relative path.
    pub path: String,
    /// The merge base's bytes (`:1:path`).
    pub base: Option<String>,
    /// The current branch's bytes (`:2:path`).
    pub ours: Option<String>,
    /// The other branch's bytes (`:3:path`).
    pub theirs: Option<String>,
}

impl Conflict {
    /// How the path is classified from its extension.
    pub fn kind(&self) -> ConflictKind {
        if self.path.to_ascii_lowercase().ends_with(".json") {
            ConflictKind::Json
        } else if self.path.to_ascii_lowercase().ends_with(".css") {
            ConflictKind::Css
        } else {
            ConflictKind::Text
        }
    }

    /// The stages Git holds for this path, in stage order.
    pub fn stages(&self) -> Vec<ConflictStage> {
        let mut stages = Vec::new();
        for stage in ConflictStage::ALL {
            if self.text(stage).is_some() {
                stages.push(stage);
            }
        }
        stages
    }

    /// One side's bytes, when Git holds that stage.
    pub fn text(&self, stage: ConflictStage) -> Option<&str> {
        match stage {
            ConflictStage::Base => self.base.as_deref(),
            ConflictStage::Ours => self.ours.as_deref(),
            ConflictStage::Theirs => self.theirs.as_deref(),
        }
    }

    /// The conflict in one line.
    pub fn render(&self) -> String {
        let stages: Vec<&str> = self.stages().into_iter().map(ConflictStage::name).collect();
        format!(
            "{} ({}, stages {})",
            self.path,
            self.kind().name(),
            stages.join("+")
        )
    }

    /// The text view, requiring both `ours` and `theirs` and keeping every side whole.
    pub fn text_conflict(&self) -> Result<TextConflict, String> {
        let ours = self.ours.as_ref().ok_or_else(|| {
            format!(
                "{} has no ours side; it cannot be resolved as text",
                self.path
            )
        })?;
        let theirs = self.theirs.as_ref().ok_or_else(|| {
            format!(
                "{} has no theirs side; it cannot be resolved as text",
                self.path
            )
        })?;
        Ok(TextConflict::new(
            self.path.clone(),
            self.base.clone().unwrap_or_default(),
            ours.clone(),
            theirs.clone(),
        ))
    }

    /// The structured design-layer diff, when the path is JSON or CSS.
    ///
    /// A side that cannot be parsed is reported as such rather than guessed at; text paths return
    /// `None` and use [`TextConflict`] instead.
    pub fn structured_diff(&self) -> Option<StructuredDiff> {
        let format = match self.kind() {
            ConflictKind::Json => StructuredFormat::Json,
            ConflictKind::Css => StructuredFormat::Css,
            ConflictKind::Text => return None,
        };
        let mut sides: [Option<Vec<StructuredEntry>>; 3] = [None, None, None];
        let mut unparsed = Vec::new();
        for (index, (stage, text)) in [
            (ConflictStage::Base, &self.base),
            (ConflictStage::Ours, &self.ours),
            (ConflictStage::Theirs, &self.theirs),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(text) = text else { continue };
            let parsed = match format {
                StructuredFormat::Json => json_entries(text),
                StructuredFormat::Css => css_entries(text),
            };
            match parsed {
                Some(entries) => sides[index] = Some(entries),
                None => unparsed.push(stage.name()),
            }
        }
        if !unparsed.is_empty() {
            return Some(StructuredDiff {
                format,
                rows: Vec::new(),
                parsed: false,
                note: Some(format!(
                    "{} side(s) could not be parsed as {}; no structured rows are guessed",
                    unparsed.join(", "),
                    format.name()
                )),
            });
        }
        Some(compare_structured(format, &sides))
    }
}

/// A design-layer format whose conflict can be compared by key rather than by line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StructuredFormat {
    /// A JSON document.
    Json,
    /// A CSS document.
    Css,
}

impl StructuredFormat {
    /// The format's name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Css => "CSS",
        }
    }
}

/// One key a structured format holds, and the value each side gives it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StructuredRow {
    /// The key: a JSON pointer-like path, or `selector|property` for CSS.
    pub key: String,
    /// The base side's value, when the key exists there.
    pub base: Option<String>,
    /// The ours side's value, when the key exists there.
    pub ours: Option<String>,
    /// The theirs side's value, when the key exists there.
    pub theirs: Option<String>,
}

impl StructuredRow {
    /// The row in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: base {:?}, ours {:?}, theirs {:?}",
            self.key, self.base, self.ours, self.theirs
        )
    }
}

/// The three sides compared by key rather than by line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StructuredDiff {
    /// Which format was compared.
    pub format: StructuredFormat,
    /// Keys the sides do not all agree on.
    pub rows: Vec<StructuredRow>,
    /// Whether every present side parsed. When false `rows` is empty and `note` says why.
    pub parsed: bool,
    /// Why no rows were produced, when parsing failed.
    pub note: Option<String>,
}

impl StructuredDiff {
    /// The diff in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = vec![format!(
            "{} structured diff: {} differing key(s)",
            self.format.name(),
            self.rows.len()
        )];
        for row in &self.rows {
            out.push(format!("  {}", row.render()));
        }
        if let Some(note) = &self.note {
            out.push(format!("  note: {note}"));
        }
        out
    }
}

/// One span the three sides disagree about, with each side's lines kept whole.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hunk {
    /// The base lines in this span (empty for an add/add conflict).
    pub base: Vec<String>,
    /// Ours lines in this span.
    pub ours: Vec<String>,
    /// Theirs lines in this span.
    pub theirs: Vec<String>,
}

/// A line- or hunk-level selection for a text conflict.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HunkChoice {
    /// Take our side of the hunk.
    Ours,
    /// Take their side of the hunk.
    Theirs,
    /// Keep both sides, ours first.
    Both,
    /// Drop the hunk entirely.
    Omit,
    /// Take specific lines, by index into the hunk's `ours` and `theirs` lines.
    Lines {
        /// Ours line indexes, in the order they are emitted.
        ours: Vec<usize>,
        /// Theirs line indexes, in the order they are emitted.
        theirs: Vec<usize>,
    },
}

impl HunkChoice {
    /// The choice's name, as a person reads it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ours => "ours",
            Self::Theirs => "theirs",
            Self::Both => "both",
            Self::Omit => "none",
            Self::Lines { .. } => "lines",
        }
    }
}

/// One stretch of a text conflict: either all three sides agree, or they are a [`Hunk`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TextSegment {
    /// Bytes all three sides agree on; every resolution keeps them.
    Same(Vec<String>),
    /// Bytes the sides disagree about; the caller chooses.
    Conflict(Hunk),
}

/// The three full sides of a text conflict and the hunks that differ.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextConflict {
    /// The project-relative path.
    pub path: String,
    /// The merge base, or empty when Git holds no base stage.
    pub base: String,
    /// Our whole side.
    pub ours: String,
    /// Their whole side.
    pub theirs: String,
    /// Agreeing spans and conflicting hunks, in file order.
    pub segments: Vec<TextSegment>,
}

impl TextConflict {
    /// Builds the hunk view from the three sides.
    fn new(path: String, base: String, ours: String, theirs: String) -> Self {
        let base_lines = lines(&base);
        let ours_lines = lines(&ours);
        let theirs_lines = lines(&theirs);
        let segments = segments(&base_lines, &ours_lines, &theirs_lines);
        Self {
            path,
            base,
            ours,
            theirs,
            segments,
        }
    }

    /// The conflicting hunks, in file order.
    pub fn hunks(&self) -> impl Iterator<Item = &Hunk> {
        self.segments.iter().filter_map(|segment| match segment {
            TextSegment::Conflict(hunk) => Some(hunk),
            TextSegment::Same(_) => None,
        })
    }

    /// How many choices a resolution needs.
    pub fn hunk_count(&self) -> usize {
        self.hunks().count()
    }

    /// Renders a resolution from one choice per hunk.
    ///
    /// This never picks a side on its own: the choice list must cover every hunk, and specific-line
    /// selections are bounds-checked. Agreeing spans are carried through unchanged.
    pub fn render(&self, choices: &[HunkChoice]) -> Result<String, String> {
        if choices.len() != self.hunk_count() {
            let given: Vec<&str> = choices.iter().map(HunkChoice::name).collect();
            return Err(format!(
                "{} has {} conflicting hunk(s) but {} choice(s) were given ({})",
                self.path,
                self.hunk_count(),
                choices.len(),
                given.join(", ")
            ));
        }
        let mut out = String::new();
        let mut choice = 0;
        for segment in &self.segments {
            match segment {
                TextSegment::Same(lines) => {
                    for line in lines {
                        out.push_str(line);
                    }
                }
                TextSegment::Conflict(hunk) => {
                    append_choice(&mut out, hunk, &choices[choice])?;
                    choice += 1;
                }
            }
        }
        Ok(out)
    }

    /// The hunks in one line per fact, with each side's lines named.
    pub fn render_hunks(&self) -> Vec<String> {
        let mut out = vec![format!(
            "{}: {} conflicting hunk(s) in {} base line(s), {} ours line(s), {} theirs line(s)",
            self.path,
            self.hunk_count(),
            self.base.lines().count(),
            self.ours.lines().count(),
            self.theirs.lines().count()
        )];
        for (index, hunk) in self.hunks().enumerate() {
            out.push(format!(
                "hunk {index}: base {} line(s), ours {}, theirs {}",
                hunk.base.len(),
                hunk.ours.len(),
                hunk.theirs.len()
            ));
            for line in &hunk.base {
                out.push(format!("  base   {}", display_line(line)));
            }
            for line in &hunk.ours {
                out.push(format!("  ours   {}", display_line(line)));
            }
            for line in &hunk.theirs {
                out.push(format!("  theirs {}", display_line(line)));
            }
        }
        out
    }
}

/// What a resolution did, and whether the caller's regeneration step has passed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resolution {
    /// The resolved path.
    pub path: String,
    /// Whether the resolved bytes were written and staged.
    pub staged: bool,
    /// The regeneration/check result, or `None` when the caller supplied no step.
    pub regeneration: Option<Result<String, String>>,
}

impl Resolution {
    /// Whether the path is staged *and* regeneration has passed.
    ///
    /// A resolution without a regeneration step is deliberately not complete: T20.07's acceptance
    /// line is that a text resolution does not by itself say the model is valid.
    pub fn is_complete(&self) -> bool {
        self.staged && matches!(&self.regeneration, Some(Ok(_)))
    }

    /// The resolution in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = vec![format!("resolved {}: written and staged", self.path)];
        match &self.regeneration {
            None => out.push(
                "regeneration: not checked; run the generation/check step before reporting success"
                    .to_owned(),
            ),
            Some(Ok(summary)) => out.push(if summary.trim().is_empty() {
                "regeneration: passed".to_owned()
            } else {
                format!("regeneration: passed ({summary})")
            }),
            Some(Err(reason)) => out.push(format!("regeneration failed: {reason}")),
        }
        out
    }
}

/// Whether an interrupted merge can be completed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Completion {
    /// Paths Git still holds unmerged stages for, in path order.
    pub unresolved: Vec<String>,
}

impl Completion {
    /// Whether every conflicted path has been resolved and staged.
    pub fn is_ready(&self) -> bool {
        self.unresolved.is_empty()
    }

    /// The completion state in one line per fact.
    pub fn render(&self) -> Vec<String> {
        if self.is_ready() {
            vec!["conflicts complete: no unmerged path remains".to_owned()]
        } else {
            let mut out = vec![format!(
                "conflicts incomplete: {} path(s) still unmerged",
                self.unresolved.len()
            )];
            for path in &self.unresolved {
                out.push(format!("  {path}"));
            }
            out
        }
    }
}

/// One worktree row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Worktree {
    /// Its path.
    pub path: String,
    /// The commit it is at.
    pub head: String,
    /// Its branch, when it has one.
    pub branch: Option<String>,
}

/// What a branch switch did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Switched {
    /// What Git reported.
    pub message: String,
    /// The message of the stash that holds what the switch set aside, when the tree was dirty.
    pub stash: Option<String>,
}

/// A discovered repository.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Repository {
    root: PathBuf,
}

impl Repository {
    /// Finds the repository that contains `path`.
    ///
    /// The path may be a file; its directory is used. A missing `git` is reported before the
    /// repository question, so a machine without Git gets the reason that applies.
    pub fn discover(path: &Path) -> Result<Self, Discover> {
        let directory = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let probe = Command::new("git")
            .args(["--version"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Discover::GitMissing(error.to_string()))?;
        if !probe.status.success() {
            return Err(Discover::GitMissing(
                String::from_utf8_lossy(&probe.stderr).trim().to_owned(),
            ));
        }
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(["rev-parse", "--show-toplevel"])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| Discover::GitMissing(error.to_string()))?;
        if !output.status.success() {
            return Err(Discover::NotRepository(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if root.is_empty() {
            return Err(Discover::NotRepository(
                "git named no top-level directory".to_owned(),
            ));
        }
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    /// The repository's top-level directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs one Git command and answers its stdout exactly as printed, or the error Git printed.
    fn output(&self, arguments: &[&str]) -> Result<String, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["-c", "core.quotepath=false"])
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot run git: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            Err(format!(
                "git {} exited with {}: {detail}",
                arguments.first().copied().unwrap_or("?"),
                output.status
            ))
        }
    }

    /// Runs one Git command and trims the trailing newline, which is what most commands mean.
    fn run(&self, arguments: &[&str]) -> Result<String, String> {
        self.output(arguments)
            .map(|text| text.trim_end().to_owned())
    }

    /// Runs one Git command whose stdout is file content, keeping every byte Git printed.
    fn run_exact(&self, arguments: &[&str]) -> Result<String, String> {
        self.output(arguments)
    }

    /// Reads the branch, changed paths and ignored paths.
    pub fn status(&self) -> Result<Status, String> {
        let text = self.run(&["status", "--porcelain=v1", "--branch", "--ignored=matching"])?;
        let mut branch = "HEAD".to_owned();
        let mut entries = Vec::new();
        let mut ignored = Vec::new();
        for line in text.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                let name = header.split("...").next().unwrap_or(header);
                branch = if name.starts_with("No commits yet on ") {
                    name.trim_start_matches("No commits yet on ").to_owned()
                } else {
                    name.to_owned()
                };
                continue;
            }
            if let Some(ignored_path) = line.strip_prefix("!! ") {
                ignored.push(ignored_path.to_owned());
                continue;
            }
            let mut characters = line.chars();
            let index = characters.next().unwrap_or(' ');
            let worktree = characters.next().unwrap_or(' ');
            let path = line.chars().skip(3).collect::<String>();
            if !path.is_empty() {
                entries.push(Entry {
                    index,
                    worktree,
                    path,
                });
            }
        }
        Ok(Status {
            branch,
            entries,
            ignored,
        })
    }

    /// The unstaged diff, or one path's unstaged diff.
    pub fn diff(&self, path: Option<&str>) -> Result<String, String> {
        let mut arguments = vec!["diff", "--no-color"];
        if let Some(path) = path {
            arguments.push("--");
            arguments.push(path);
        }
        self.run(&arguments)
    }

    /// The staged diff, or one path's staged diff.
    pub fn staged_diff(&self, path: Option<&str>) -> Result<String, String> {
        let mut arguments = vec!["diff", "--cached", "--no-color"];
        if let Some(path) = path {
            arguments.push("--");
            arguments.push(path);
        }
        self.run(&arguments)
    }

    /// Every unresolved conflict, with all three sides read out of the index.
    ///
    /// `git ls-files -u -z` says which stages exist per path; each is then read with
    /// `git show :1:path`, `:2:path` or `:3:path`. Arguments only, never a shell, and the exact
    /// bytes `git show` printed are kept (a trailing newline is part of a file).
    pub fn conflicts(&self) -> Result<Vec<Conflict>, String> {
        let text = self.run(&["ls-files", "-u", "-z"])?;
        let mut stages: BTreeMap<String, [bool; 3]> = BTreeMap::new();
        for record in text.split('\0') {
            if record.is_empty() {
                continue;
            }
            let Some((metadata, path)) = record.split_once('\t') else {
                return Err(format!(
                    "git ls-files -u printed a row without a path: {record:?}"
                ));
            };
            let mut fields = metadata.split_whitespace();
            let mode = fields.next();
            let object = fields.next();
            let stage = fields.next();
            if mode.is_none() || object.is_none() {
                return Err(format!(
                    "git ls-files -u printed an incomplete row: {record:?}"
                ));
            }
            let stage = stage
                .and_then(ConflictStage::parse)
                .ok_or_else(|| format!("git ls-files -u printed an unknown stage: {record:?}"))?;
            stages.entry(path.to_owned()).or_default()[stage_index(stage)] = true;
        }
        let mut conflicts = Vec::with_capacity(stages.len());
        for (path, present) in stages {
            let mut conflict = Conflict {
                path: path.clone(),
                base: None,
                ours: None,
                theirs: None,
            };
            for stage in ConflictStage::ALL {
                if !present[stage_index(stage)] {
                    continue;
                }
                let reference = format!(":{}:{path}", stage.number());
                let text = self.run_exact(&["show", &reference])?;
                match stage {
                    ConflictStage::Base => conflict.base = Some(text),
                    ConflictStage::Ours => conflict.ours = Some(text),
                    ConflictStage::Theirs => conflict.theirs = Some(text),
                }
            }
            conflicts.push(conflict);
        }
        Ok(conflicts)
    }

    /// One unresolved conflict by path.
    pub fn conflict(&self, path: &str) -> Result<Conflict, String> {
        self.conflicts()?
            .into_iter()
            .find(|conflict| conflict.path == path)
            .ok_or_else(|| format!("{path:?} is not an unresolved conflict"))
    }

    /// Whether the merge can be completed: no path may still have unmerged stages.
    pub fn completion(&self) -> Result<Completion, String> {
        Ok(Completion {
            unresolved: self
                .conflicts()?
                .into_iter()
                .map(|conflict| conflict.path)
                .collect(),
        })
    }

    /// Resolves a JSON/CSS conflict from an explicit, caller-written document.
    ///
    /// This never merges the three sides. The document must be non-empty and must parse as the
    /// path's format; it is then written and staged, and `regenerate` is asked to prove the model
    /// still generates. A failed regeneration is kept in the answer, not hidden.
    pub fn resolve_document(
        &self,
        path: &str,
        document: &str,
        regenerate: Option<&mut dyn FnMut() -> Result<String, String>>,
    ) -> Result<Resolution, String> {
        let conflict = self.conflict(path)?;
        if document.trim().is_empty() {
            return Err(format!(
                "resolving {path:?} needs an explicit document; an empty document is not a resolution"
            ));
        }
        match conflict.kind() {
            ConflictKind::Json => {
                if json_entries(document).is_none() {
                    return Err(format!(
                        "the resolved document for {path:?} is not valid JSON; no side was merged in"
                    ));
                }
            }
            ConflictKind::Css => {
                if css_entries(document).is_none() {
                    return Err(format!(
                        "the resolved document for {path:?} is not valid CSS; no side was merged in"
                    ));
                }
            }
            ConflictKind::Text => {
                return Err(format!(
                    "{path:?} is a text conflict; use a line/hunk selection or an explicit text file"
                ));
            }
        }
        self.write_resolution(path, document, regenerate)
    }

    /// Resolves a text conflict from one choice per hunk, then writes and stages the result.
    pub fn resolve_text(
        &self,
        conflict: &TextConflict,
        choices: &[HunkChoice],
        regenerate: Option<&mut dyn FnMut() -> Result<String, String>>,
    ) -> Result<Resolution, String> {
        let resolved = conflict.render(choices)?;
        self.resolve_text_file(&conflict.path, &resolved, regenerate)
    }

    /// Resolves a text conflict from a whole caller-written file.
    ///
    /// A generated file is protected even on this path: the resolved text must keep the generated
    /// marker, every hand-written region must parse, and no hand-written body may differ between the
    /// sides or be changed or dropped by the resolution. That is what stops a text resolution from
    /// quietly claiming ownership of hand-written bytes.
    pub fn resolve_text_file(
        &self,
        path: &str,
        document: &str,
        regenerate: Option<&mut dyn FnMut() -> Result<String, String>>,
    ) -> Result<Resolution, String> {
        let conflict = self.conflict(path)?;
        if conflict.kind() != ConflictKind::Text {
            return Err(format!(
                "{path:?} is a {} conflict; use an explicit document rather than a text resolution",
                conflict.kind().name()
            ));
        }
        protect_generated_zones(&conflict, document)?;
        self.write_resolution(path, document, regenerate)
    }

    /// Writes and stages a resolution, then runs the caller's regeneration step once.
    fn write_resolution(
        &self,
        path: &str,
        text: &str,
        regenerate: Option<&mut dyn FnMut() -> Result<String, String>>,
    ) -> Result<Resolution, String> {
        let target = self.target(path)?;
        if let Some(parent) = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
        }
        std::fs::write(&target, text)
            .map_err(|error| format!("{} could not be written: {error}", target.display()))?;
        self.stage(path)?;
        let regeneration = regenerate.map(|step| step());
        Ok(Resolution {
            path: path.to_owned(),
            staged: true,
            regeneration,
        })
    }

    /// The repository-relative path a resolution may write, refusing escapes.
    fn target(&self, path: &str) -> Result<PathBuf, String> {
        if path.trim().is_empty() {
            return Err("a path is required".to_owned());
        }
        let relative = Path::new(path);
        if relative.is_absolute() {
            return Err(format!(
                "{path:?} is absolute; a repository path must be relative to {}",
                self.root.display()
            ));
        }
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "{path:?} escapes the repository; refusing to write it"
            ));
        }
        Ok(self.root.join(relative))
    }

    /// Adds one path to the index.
    pub fn stage(&self, path: &str) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("a path is required".to_owned());
        }
        self.run(&["add", "--", path]).map(|_| ())
    }

    /// Takes one path out of the index, leaving the worktree file alone.
    pub fn unstage(&self, path: &str) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("a path is required".to_owned());
        }
        self.run(&["restore", "--staged", "--", path]).map(|_| ())
    }

    /// Commits the index with an explicit message.
    ///
    /// An empty message is refused before Git runs, and Git itself refuses an empty index.
    pub fn commit(&self, message: &str) -> Result<String, String> {
        if message.trim().is_empty() {
            return Err("a commit message is required".to_owned());
        }
        self.run(&["commit", "-m", message])
    }

    /// The local branch names.
    pub fn branches(&self) -> Result<Vec<String>, String> {
        let text = self.run(&["branch", "--format=%(refname:short)"])?;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Refuses a branch-changing operation while the worktree has changes.
    fn require_clean(&self, operation: &str) -> Result<Status, String> {
        let status = self.status()?;
        if !status.is_clean() {
            return Err(format!(
                "{operation} needs a clean worktree, and {} path(s) changed on {}",
                status.entries.len(),
                status.branch
            ));
        }
        Ok(status)
    }

    /// Creates and switches to a branch, refusing a dirty worktree.
    pub fn create_branch(&self, name: &str) -> Result<String, String> {
        if name.trim().is_empty() {
            return Err("a branch name is required".to_owned());
        }
        self.require_clean("creating a branch")?;
        self.run(&["switch", "-c", name])
            .map(|_| format!("created and switched to {name}"))
    }

    /// Switches branches, refusing a dirty worktree.
    pub fn switch_branch(&self, name: &str) -> Result<String, String> {
        if name.trim().is_empty() {
            return Err("a branch name is required".to_owned());
        }
        self.require_clean("switching branches")?;
        self.run(&["switch", name])
            .map(|_| format!("switched to {name}"))
    }

    /// Switches branches, setting a dirty worktree aside first.
    ///
    /// The stash is not an afterthought: it is named in the answer so the work is findable, and a
    /// switch that fails after the stash is popped again. If that restore fails too, the error says
    /// where the work still is instead of reporting only the switch failure.
    pub fn switch_branch_stashing(&self, name: &str) -> Result<Switched, String> {
        if name.trim().is_empty() {
            return Err("a branch name is required".to_owned());
        }
        let status = self.status()?;
        if status.is_clean() {
            return self.run(&["switch", name]).map(|_| Switched {
                message: format!("switched to {name}"),
                stash: None,
            });
        }
        let message = format!("rustage: switch to {name}");
        self.stash(&message)?;
        match self.run(&["switch", name]) {
            Ok(_) => Ok(Switched {
                message: format!("switched to {name}"),
                stash: Some(message),
            }),
            Err(reason) => match self.stash_pop(0) {
                Ok(_) => Err(format!(
                    "{reason}; the {} path(s) set aside for the switch were restored",
                    status.entries.len()
                )),
                Err(restore) => Err(format!(
                    "{reason}; the {} path(s) are still in stash@{{0}} ({message}) but restoring \
                     them failed: {restore}",
                    status.entries.len()
                )),
            },
        }
    }

    /// The worktrees Git reports.
    pub fn worktrees(&self) -> Result<Vec<Worktree>, String> {
        let text = self.run(&["worktree", "list", "--porcelain"])?;
        let mut out = Vec::new();
        let mut path = String::new();
        let mut head = String::new();
        let mut branch = None;
        for line in text.lines().chain(std::iter::once("")) {
            if line.is_empty() {
                if !path.is_empty() {
                    out.push(Worktree {
                        path: path.clone(),
                        head: head.clone(),
                        branch: branch.clone(),
                    });
                }
                path.clear();
                head.clear();
                branch = None;
                continue;
            }
            if let Some(value) = line.strip_prefix("worktree ") {
                path = value.to_owned();
            } else if let Some(value) = line.strip_prefix("HEAD ") {
                head = value.to_owned();
            } else if let Some(value) = line.strip_prefix("branch ") {
                branch = Some(value.trim_start_matches("refs/heads/").to_owned());
            }
        }
        Ok(out)
    }

    /// Adds a worktree at `path` for an existing branch.
    pub fn add_worktree(&self, path: &str, branch: &str) -> Result<String, String> {
        if path.trim().is_empty() || branch.trim().is_empty() {
            return Err("a worktree path and branch are required".to_owned());
        }
        if !self.branches()?.iter().any(|name| name == branch) {
            return Err(format!("no branch {branch:?} exists"));
        }
        self.run(&["worktree", "add", path, branch])
            .map(|_| format!("added worktree {path} on {branch}"))
    }

    /// Stashes the worktree and index, refusing a clean tree.
    pub fn stash(&self, message: &str) -> Result<String, String> {
        let status = self.status()?;
        if status.is_clean() {
            return Err("there is nothing to stash".to_owned());
        }
        let message = if message.trim().is_empty() {
            "rustage"
        } else {
            message
        };
        self.run(&["stash", "push", "-u", "-m", message])
            .map(|_| format!("stashed {} path(s)", status.entries.len()))
    }

    /// The stash entries.
    pub fn stash_list(&self) -> Result<Vec<String>, String> {
        let text = self.run(&["stash", "list"])?;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Pops one stash entry (default `0`), removing it on success.
    pub fn stash_pop(&self, index: u32) -> Result<String, String> {
        self.run(&["stash", "pop", &format!("stash@{{{index}}}")])
            .map(|_| format!("popped stash@{{{index}}}"))
    }

    /// Applies one stash entry without removing it.
    pub fn stash_apply(&self, index: u32) -> Result<String, String> {
        self.run(&["stash", "apply", &format!("stash@{{{index}}}")])
            .map(|_| format!("applied stash@{{{index}}}"))
    }

    /// Merges a branch without committing an empty fast-forward, reporting a conflict.
    ///
    /// A conflict is still the Git error the caller needs, but the refusal names the unmerged paths
    /// and the resolution step rather than leaving a person to run `git status` to find out why.
    pub fn merge(&self, branch: &str) -> Result<String, String> {
        match self.run(&["merge", "--no-ff", "--no-edit", branch]) {
            Ok(summary) => Ok(summary),
            Err(reason) => {
                let conflicts = self.conflicts().unwrap_or_default();
                if conflicts.is_empty() {
                    Err(reason)
                } else {
                    let paths: Vec<&str> = conflicts
                        .iter()
                        .map(|conflict| conflict.path.as_str())
                        .collect();
                    Err(format!(
                        "{reason}\nmerge left {} unresolved path(s): {}; resolve each, then run \
                         regeneration before committing (merge-abort restores the pre-merge state)",
                        conflicts.len(),
                        paths.join(", ")
                    ))
                }
            }
        }
    }

    /// Fetches a remote without touching the worktree. This is the only network operation that is
    /// safe without a confirmation.
    pub fn fetch(&self, remote: &str) -> Result<String, String> {
        let remote = if remote.trim().is_empty() {
            "origin"
        } else {
            remote
        };
        self.run(&["fetch", remote])
            .map(|_| format!("fetched {remote}"))
    }

    /// Pulls with `--ff-only`, so a divergent branch is a refusal rather than a merge.
    pub fn pull(&self, remote: &str) -> Result<String, String> {
        let remote = if remote.trim().is_empty() {
            "origin"
        } else {
            remote
        };
        self.run(&["pull", "--ff-only", remote])
            .map(|_| format!("pulled {remote} (fast-forward only)"))
    }

    /// Pushes an explicit branch after the caller confirmed it.
    ///
    /// `confirmed` is not a default: without it this refuses before Git runs. There is no force
    /// flag and no automatic push after commit.
    pub fn push(&self, remote: &str, branch: &str, confirmed: bool) -> Result<String, String> {
        if !confirmed {
            return Err("push needs an explicit confirmation".to_owned());
        }
        if branch.trim().is_empty() {
            return Err("a branch is required for push".to_owned());
        }
        let remote = if remote.trim().is_empty() {
            "origin"
        } else {
            remote
        };
        self.run(&["push", remote, branch])
            .map(|_| format!("pushed {branch} to {remote}"))
    }

    /// Aborts an in-progress merge.
    pub fn merge_abort(&self) -> Result<String, String> {
        self.run(&["merge", "--abort"])
            .map(|_| "merge aborted".to_owned())
    }

    /// Rebases onto a branch, reporting a conflict.
    pub fn rebase(&self, branch: &str) -> Result<String, String> {
        self.run(&["rebase", branch])
            .map(|_| format!("rebased onto {branch}"))
    }

    /// Aborts an in-progress rebase.
    pub fn rebase_abort(&self) -> Result<String, String> {
        self.run(&["rebase", "--abort"])
            .map(|_| "rebase aborted".to_owned())
    }

    /// Cherry-picks one revision, reporting a conflict.
    pub fn cherry_pick(&self, revision: &str) -> Result<String, String> {
        self.run(&["cherry-pick", revision])
            .map(|_| format!("cherry-picked {revision}"))
    }

    /// Aborts an in-progress cherry-pick.
    pub fn cherry_pick_abort(&self) -> Result<String, String> {
        self.run(&["cherry-pick", "--abort"])
            .map(|_| "cherry-pick aborted".to_owned())
    }
}

/// The array position of a stage in a grouped `ls-files -u` row.
const fn stage_index(stage: ConflictStage) -> usize {
    match stage {
        ConflictStage::Base => 0,
        ConflictStage::Ours => 1,
        ConflictStage::Theirs => 2,
    }
}

/// A key and its value in a structured design format.
type StructuredEntry = (String, String);

/// One value a key holds across the three sides.
fn compare_structured(
    format: StructuredFormat,
    sides: &[Option<Vec<StructuredEntry>>; 3],
) -> StructuredDiff {
    let maps: Vec<Option<BTreeMap<String, String>>> = sides
        .iter()
        .map(|side| {
            side.as_ref()
                .map(|entries| entries.iter().cloned().collect())
        })
        .collect();
    let mut keys = BTreeSet::new();
    for map in maps.iter().flatten() {
        keys.extend(map.keys().cloned());
    }
    let mut rows = Vec::new();
    for key in keys {
        let value = |index: usize| maps[index].as_ref().and_then(|map| map.get(&key).cloned());
        let base = value(0);
        let ours = value(1);
        let theirs = value(2);
        if base != ours || ours != theirs {
            rows.push(StructuredRow {
                key,
                base,
                ours,
                theirs,
            });
        }
    }
    StructuredDiff {
        format,
        rows,
        parsed: true,
        note: None,
    }
}

/// A conflict hunk the sides disagree about, as line ranges into base and one side.
#[derive(Clone, Copy)]
struct DiffChange {
    base_start: usize,
    base_end: usize,
    other_start: usize,
    other_end: usize,
}

/// The largest LCS table this module builds; beyond it one whole-file hunk is honest about the cost.
const MAX_DIFF_CELLS: usize = 1_000_000;

/// Splits text into lines while keeping each line's ending.
fn lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_owned).collect()
}

/// The spans where `other` differs from `base`.
fn line_changes(base: &[String], other: &[String]) -> Vec<DiffChange> {
    if base.len().saturating_mul(other.len()) > MAX_DIFF_CELLS {
        return if base.is_empty() && other.is_empty() {
            Vec::new()
        } else {
            vec![DiffChange {
                base_start: 0,
                base_end: base.len(),
                other_start: 0,
                other_end: other.len(),
            }]
        };
    }
    let n = base.len();
    let m = other.len();
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if base[i] == other[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut start: Option<(usize, usize)> = None;
    while i < n && j < m {
        if base[i] == other[j] {
            if let Some((base_start, other_start)) = start.take() {
                changes.push(DiffChange {
                    base_start,
                    base_end: i,
                    other_start,
                    other_end: j,
                });
            }
            i += 1;
            j += 1;
        } else {
            if start.is_none() {
                start = Some((i, j));
            }
            if lcs[i + 1][j] >= lcs[i][j + 1] {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    if let Some((base_start, other_start)) = start.take() {
        changes.push(DiffChange {
            base_start,
            base_end: i,
            other_start,
            other_end: j,
        });
    }
    if i < n || j < m {
        changes.push(DiffChange {
            base_start: i,
            base_end: n,
            other_start: j,
            other_end: m,
        });
    }
    changes.retain(|change| {
        change.base_start != change.base_end || change.other_start != change.other_end
    });
    changes
}

/// Builds agreeing spans and conflicting hunks from the three sides.
fn segments(base: &[String], ours: &[String], theirs: &[String]) -> Vec<TextSegment> {
    let ours_changes = line_changes(base, ours);
    let theirs_changes = line_changes(base, theirs);
    let mut ranges: Vec<(usize, usize)> = ours_changes
        .iter()
        .chain(&theirs_changes)
        .map(|change| (change.base_start, change.base_end))
        .collect();
    ranges.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
            last.1 = last.1.max(end);
            continue;
        }
        merged.push((start, end));
    }

    let mut out = Vec::new();
    let mut cursor = 0;
    for (start, end) in merged {
        if cursor < start {
            out.push(TextSegment::Same(base[cursor..start].to_vec()));
        }
        let base_slice = base[start..end].to_vec();
        let ours_slice = materialize(base, ours, &ours_changes, start, end);
        let theirs_slice = materialize(base, theirs, &theirs_changes, start, end);
        if ours_slice == base_slice && theirs_slice == base_slice {
            out.push(TextSegment::Same(base_slice));
        } else {
            out.push(TextSegment::Conflict(Hunk {
                base: base_slice,
                ours: ours_slice,
                theirs: theirs_slice,
            }));
        }
        cursor = end;
    }
    if cursor < base.len() {
        out.push(TextSegment::Same(base[cursor..].to_vec()));
    }
    out
}

/// Rebuilds one side's bytes for a base span, applying the changes that fall inside it.
fn materialize(
    base: &[String],
    other: &[String],
    changes: &[DiffChange],
    start: usize,
    end: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = start;
    let mut index = 0;
    while index < changes.len() && changes[index].base_start < start {
        index += 1;
    }
    loop {
        if index < changes.len() && changes[index].base_start == at {
            let change = changes[index];
            out.extend(other[change.other_start..change.other_end].iter().cloned());
            at = change.base_end;
            index += 1;
            if at > end {
                break;
            }
            continue;
        }
        if at < end {
            out.push(base[at].clone());
            at += 1;
            continue;
        }
        break;
    }
    out
}

/// Emits one hunk choice, refusing an index outside the hunk.
fn append_choice(out: &mut String, hunk: &Hunk, choice: &HunkChoice) -> Result<(), String> {
    match choice {
        HunkChoice::Ours => push_all(out, &hunk.ours),
        HunkChoice::Theirs => push_all(out, &hunk.theirs),
        HunkChoice::Both => {
            push_all(out, &hunk.ours);
            push_all(out, &hunk.theirs);
        }
        HunkChoice::Omit => {}
        HunkChoice::Lines { ours, theirs } => {
            for index in ours {
                let line = hunk.ours.get(*index).ok_or_else(|| {
                    format!(
                        "ours line index {index} is outside a hunk with {} line(s)",
                        hunk.ours.len()
                    )
                })?;
                out.push_str(line);
            }
            for index in theirs {
                let line = hunk.theirs.get(*index).ok_or_else(|| {
                    format!(
                        "theirs line index {index} is outside a hunk with {} line(s)",
                        hunk.theirs.len()
                    )
                })?;
                out.push_str(line);
            }
        }
    }
    Ok(())
}

/// Appends every line verbatim.
fn push_all(out: &mut String, lines: &[String]) {
    for line in lines {
        out.push_str(line);
    }
}

/// One line for a hunk listing, without adding a line ending of its own.
fn display_line(line: &str) -> String {
    let trimmed = line.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() && !line.is_empty() {
        "(blank line)".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Refuses a text resolution that would cross a generated file's hand-written boundary.
fn protect_generated_zones(conflict: &Conflict, resolved: &str) -> Result<(), String> {
    use crate::app::generate::artifact::{self, RegionKind};

    let sides = [
        ("base", &conflict.base),
        ("ours", &conflict.ours),
        ("theirs", &conflict.theirs),
    ];
    let generated: Vec<(&str, &str)> = sides
        .into_iter()
        .filter_map(|(name, text)| text.as_deref().map(|text| (name, text)))
        .filter(|(_, text)| artifact::is_generated(text))
        .collect();
    if generated.is_empty() {
        return Ok(());
    }
    if !artifact::is_generated(resolved) {
        return Err(format!(
            "resolving {:?} would drop the generated marker; a text resolution cannot claim bytes a \
             person owns",
            conflict.path
        ));
    }
    let resolved_regions = artifact::regions(resolved).map_err(|reason| {
        format!(
            "the resolved {:?} does not have provable hand-written boundaries: {reason}",
            conflict.path
        )
    })?;
    let resolved_bodies = handwritten_bodies(resolved, &resolved_regions, &conflict.path)?;

    // A body that differs between the sides is not a text-resolution decision: choosing it would
    // silently discard one person's bytes. Identical bodies must survive verbatim.
    let mut expected: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (name, side) in generated {
        let side_regions = artifact::regions(side).map_err(|reason| {
            format!(
                "the {name} side of {:?} does not have provable hand-written boundaries: {reason}",
                conflict.path
            )
        })?;
        for region in &side_regions {
            if region.kind != RegionKind::Handwritten || region.name.is_empty() {
                continue;
            }
            let body = region.content(side).to_owned();
            match expected.get_mut(&region.name) {
                None => {
                    expected.insert(region.name.clone(), Some(body));
                }
                Some(existing) if existing.as_deref() != Some(body.as_str()) => {
                    *existing = None;
                }
                Some(_) => {}
            }
        }
    }
    for (name, body) in expected {
        let Some(body) = body else {
            return Err(format!(
                "the hand-written region {name:?} in {:?} differs between the conflicting sides; a \
                 text resolution cannot choose those bytes (regenerate or resolve through the design \
                 model)",
                conflict.path
            ));
        };
        match resolved_bodies.get(&name) {
            Some(found) if found == &body => {}
            Some(_) => {
                return Err(format!(
                    "resolving {:?} changed the hand-written region {name:?}; regeneration owns only \
                     the generated zone",
                    conflict.path
                ));
            }
            None => {
                return Err(format!(
                    "resolving {:?} dropped the hand-written region {name:?}; a text resolution cannot \
                     remove bytes a person owns",
                    conflict.path
                ));
            }
        }
    }
    Ok(())
}

/// The hand-written bodies a generated text holds, by region name.
fn handwritten_bodies(
    text: &str,
    regions: &[crate::app::generate::artifact::Region],
    path: &str,
) -> Result<BTreeMap<String, String>, String> {
    use crate::app::generate::artifact::RegionKind;

    let mut bodies = BTreeMap::new();
    for region in regions {
        if region.kind != RegionKind::Handwritten || region.name.is_empty() {
            continue;
        }
        if bodies
            .insert(region.name.clone(), region.content(text).to_owned())
            .is_some()
        {
            return Err(format!(
                "the resolved {path:?} names the hand-written region {:?} twice; the boundary is not \
                 provable",
                region.name
            ));
        }
    }
    Ok(bodies)
}

/// A tolerant JSON reader that flattens leaves into pointer-like keys.
///
/// This is not a general JSON library; it answers "which keys and values does each side hold?" for
/// the design-layer diff. A malformed document returns `None`, so the caller can say the side did
/// not parse rather than guessing at rows.
fn json_entries(text: &str) -> Option<Vec<StructuredEntry>> {
    let mut parser = JsonParser { text, at: 0 };
    parser.skip_ws();
    if parser.at == text.len() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    parser.value("", &mut out)?;
    parser.skip_ws();
    (parser.at == text.len()).then_some(out)
}

struct JsonParser<'a> {
    text: &'a str,
    at: usize,
}

impl JsonParser<'_> {
    fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    fn byte(&self) -> Option<u8> {
        self.bytes().get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.byte(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, path: &str, out: &mut Vec<StructuredEntry>) -> Option<()> {
        self.skip_ws();
        match self.byte()? {
            b'{' => self.object(path, out),
            b'[' => self.array(path, out),
            b'"' => {
                let start = self.at;
                self.string()?;
                out.push((root_path(path), self.text[start..self.at].to_owned()));
                Some(())
            }
            b't' => self.literal("true", path, out),
            b'f' => self.literal("false", path, out),
            b'n' => self.literal("null", path, out),
            _ => self.number(path, out),
        }
    }

    fn object(&mut self, path: &str, out: &mut Vec<StructuredEntry>) -> Option<()> {
        self.at += 1;
        self.skip_ws();
        if self.byte()? == b'}' {
            self.at += 1;
            out.push((root_path(path), "{}".to_owned()));
            return Some(());
        }
        loop {
            self.skip_ws();
            let key_start = self.at;
            self.string()?;
            let key = self.text[key_start..self.at].trim_matches('"').to_owned();
            self.skip_ws();
            if self.byte()? != b':' {
                return None;
            }
            self.at += 1;
            let child = if path.is_empty() {
                key
            } else {
                format!("{path}.{key}")
            };
            self.value(&child, out)?;
            self.skip_ws();
            match self.byte()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self, path: &str, out: &mut Vec<StructuredEntry>) -> Option<()> {
        self.at += 1;
        self.skip_ws();
        if self.byte()? == b']' {
            self.at += 1;
            out.push((root_path(path), "[]".to_owned()));
            return Some(());
        }
        let mut index = 0usize;
        loop {
            let child = if path.is_empty() {
                format!("$[{index}]")
            } else {
                format!("{path}[{index}]")
            };
            self.value(&child, out)?;
            index += 1;
            self.skip_ws();
            match self.byte()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<()> {
        if self.byte()? != b'"' {
            return None;
        }
        self.at += 1;
        while let Some(byte) = self.byte() {
            match byte {
                b'\\' => self.at += 2,
                b'"' => {
                    self.at += 1;
                    return Some(());
                }
                _ => self.at += 1,
            }
        }
        None
    }

    fn literal(&mut self, word: &str, path: &str, out: &mut Vec<StructuredEntry>) -> Option<()> {
        if !self.text[self.at..].starts_with(word) {
            return None;
        }
        self.at += word.len();
        out.push((root_path(path), word.to_owned()));
        Some(())
    }

    fn number(&mut self, path: &str, out: &mut Vec<StructuredEntry>) -> Option<()> {
        let start = self.at;
        if self.byte() == Some(b'-') {
            self.at += 1;
        }
        while matches!(
            self.byte(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        if self.at == start {
            return None;
        }
        out.push((root_path(path), self.text[start..self.at].to_owned()));
        Some(())
    }
}

fn root_path(path: &str) -> String {
    if path.is_empty() {
        "$".to_owned()
    } else {
        path.to_owned()
    }
}

/// Reads CSS rules into `selector|property` keys.
///
/// Comment removal and a brace-depth check keep a malformed document from becoming invented rows;
/// nested rules return `None` rather than guessing which selector owns a declaration.
fn css_entries(text: &str) -> Option<Vec<StructuredEntry>> {
    let text = strip_css_comments(text);
    let mut entries = Vec::new();
    let mut rest = text.as_str();
    loop {
        let current = rest.trim_start();
        if current.is_empty() {
            return Some(entries);
        }
        let open = current.find('{')?;
        let selector = current[..open].trim();
        if selector.is_empty() {
            return None;
        }
        let body = &current[open + 1..];
        let mut depth = 1usize;
        let mut nested = false;
        let mut close = None;
        for (index, character) in body.char_indices() {
            match character {
                '{' => {
                    depth += 1;
                    nested = true;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let close = close?;
        if nested {
            return None;
        }
        for declaration in body[..close].split(';') {
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            let (property, value) = declaration.split_once(':')?;
            let property = property.trim();
            if property.is_empty() {
                return None;
            }
            entries.push((format!("{selector}|{property}"), value.trim().to_owned()));
        }
        rest = &body[close + 1..];
    }
}

/// Removes CSS comments so a declaration inside one is not read as a rule.
fn strip_css_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// One commit in the history, as Git printed it (T20.04).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Commit {
    /// The full hash.
    pub hash: String,
    /// The author's name.
    pub author: String,
    /// The author's e-mail.
    pub email: String,
    /// The author time, in seconds since the Unix epoch (UTC).
    pub time: i64,
    /// The first line of the message.
    pub subject: String,
}

impl Commit {
    /// The first seven characters of the hash.
    pub fn short(&self) -> &str {
        let end = self.hash.len().min(7);
        &self.hash[..end]
    }
}

/// What restoring one path from a revision would do, without doing it (T20.04).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RestorePreview {
    /// The revision the content comes from.
    pub revision: String,
    /// The path, as Git knows it.
    pub path: String,
    /// The bytes the revision holds.
    pub bytes: usize,
    /// Whether the working copy differs from them.
    pub differs: bool,
    /// Whether the working copy has no such file.
    pub missing: bool,
}

impl RestorePreview {
    /// The preview in lines: what would happen, then that nothing was written.
    pub fn render(&self) -> Vec<String> {
        let state = if self.missing {
            "the working copy has no such file".to_owned()
        } else if self.differs {
            "the working copy differs".to_owned()
        } else {
            "the working copy is identical".to_owned()
        };
        vec![
            format!(
                "restore preview: {} from {} ({} byte(s))",
                self.path, self.revision, self.bytes
            ),
            state,
            "preview only: nothing was written".to_owned(),
        ]
    }
}

impl Repository {
    /// The commit history, newest first, at most `limit` entries.
    pub fn log(&self, limit: Option<usize>) -> Result<Vec<Commit>, String> {
        let limit = limit.unwrap_or(20);
        if limit == 0 {
            return Err("a history needs at least one commit".to_owned());
        }
        let text = self.run(&[
            "log",
            "--no-color",
            &format!("--max-count={limit}"),
            "--format=%H%x1f%an%x1f%ae%x1f%at%x1f%s",
        ])?;
        let mut commits = Vec::new();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\u{1f}').collect();
            let [hash, author, email, time, subject] = fields.as_slice() else {
                return Err(format!(
                    "git log printed {} field(s), not 5: {line:?}",
                    fields.len()
                ));
            };
            let time: i64 = time
                .parse()
                .map_err(|_| format!("the author time {time:?} is not a number"))?;
            commits.push(Commit {
                hash: (*hash).to_owned(),
                author: (*author).to_owned(),
                email: (*email).to_owned(),
                time,
                subject: (*subject).to_owned(),
            });
        }
        Ok(commits)
    }

    /// What one commit changed, or one path's content at that commit.
    ///
    /// Without a path this is `git show`, so it is a diff; with one it is the file's content at that
    /// revision. Both are read-only.
    pub fn commit_show(&self, revision: &str, path: Option<&str>) -> Result<String, String> {
        if revision.trim().is_empty() {
            return Err("a revision is needed".to_owned());
        }
        match path {
            Some(path) if path.trim().is_empty() => Err("the path is empty".to_owned()),
            Some(path) => self.run_exact(&["show", &format!("{revision}:{path}")]),
            None => self.run(&["show", "--no-color", revision]),
        }
    }

    /// The hunks of one path's unstaged text change, in the order Git printed them.
    ///
    /// A binary change has no hunks; an empty answer means there is nothing to choose from, not
    /// that the file is unchanged.
    pub fn diff_hunks(&self, path: &str) -> Result<Vec<String>, String> {
        if path.trim().is_empty() {
            return Err("a path is needed".to_owned());
        }
        let text = self.run(&["diff", "--no-color", "--", path])?;
        Ok(hunks_of(&text))
    }

    /// Stages one hunk of a path's unstaged change, leaving the rest in the worktree.
    ///
    /// `index` is 1-based, because it is a number a person reads. The patch is the file header Git
    /// itself printed plus exactly that hunk, applied with `git apply --cached`, so the index gains
    /// the selected change and the working copy is untouched.
    pub fn stage_hunk(&self, path: &str, index: usize) -> Result<String, String> {
        if path.trim().is_empty() {
            return Err("a path is needed".to_owned());
        }
        let text = self.run(&["diff", "--no-color", "--", path])?;
        let hunks = hunks_of(&text);
        if hunks.is_empty() {
            return Err(format!("{path:?} has no unstaged text change to stage"));
        }
        if index == 0 || index > hunks.len() {
            return Err(format!(
                "hunk {index} does not exist; {path:?} has {} hunk(s)",
                hunks.len()
            ));
        }
        let header: String = text
            .lines()
            .take_while(|line| !line.starts_with("@@"))
            .map(|line| format!("{line}\n"))
            .collect();
        if header.trim().is_empty() {
            return Err(format!("{path:?} has no file header to stage"));
        }
        let patch = format!("{header}{}", hunks[index - 1]);
        self.apply_to_index(&patch)?;
        Ok(format!(
            "staged hunk {index} of {path} ({} hunk(s) were unstaged)",
            hunks.len()
        ))
    }

    /// Applies a patch to the index only.
    fn apply_to_index(&self, patch: &str) -> Result<(), String> {
        use std::io::Write;
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["apply", "--cached", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run git apply: {error}"))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| "git apply got no input pipe".to_owned())?;
            stdin
                .write_all(patch.as_bytes())
                .map_err(|error| format!("cannot write the patch: {error}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("cannot wait for git apply: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(format!(
                "git apply --cached exited with {}: {stderr}",
                output.status
            ))
        }
    }

    /// What restoring `path` from `revision` would produce, without writing anything.
    pub fn restore_preview(&self, revision: &str, path: &str) -> Result<RestorePreview, String> {
        if revision.trim().is_empty() {
            return Err("a revision is needed".to_owned());
        }
        if path.trim().is_empty() {
            return Err("a path is needed".to_owned());
        }
        let content = self.run_exact(&["show", &format!("{revision}:{path}")])?;
        let working = std::fs::read(self.root.join(path));
        let (differs, missing) = match working {
            Ok(bytes) => (bytes != content.as_bytes(), false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (true, true),
            Err(error) => {
                return Err(format!("cannot read {path}: {error}"));
            }
        };
        Ok(RestorePreview {
            revision: revision.to_owned(),
            path: path.to_owned(),
            bytes: content.len(),
            differs,
            missing,
        })
    }
}

/// What a commit would do, before it is made (T20.03).
///
/// The preview is the whole of the acceptance line in one value: what is staged, what is
/// deliberately left out, which open buffers have unsaved changes, and every secret the staged
/// content contains. Nothing is staged, written or committed by building one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitPreview {
    /// The branch the commit would land on.
    pub branch: String,
    /// The paths that would go in, taken from the index only.
    pub staged: Vec<String>,
    /// Changed paths that are not staged, so they stay out of this commit.
    pub not_staged: Vec<String>,
    /// Untracked paths, which also stay out.
    pub untracked: Vec<String>,
    /// Staged paths whose open buffer has unsaved changes.
    pub dirty: Vec<String>,
    /// Secrets found in the staged content.
    pub secrets: Vec<Secret>,
    /// Staged paths whose content could not be read as text.
    pub unscanned: Vec<String>,
}

impl CommitPreview {
    /// Whether committing this would be authorized.
    ///
    /// Two things stop it: no staged path (there is nothing to commit) and a secret in the staged
    /// content. A dirty buffer is a warning, not a refusal — the person may have staged exactly what
    /// they meant to, and the preview says so out loud.
    pub fn can_commit(&self) -> bool {
        !self.staged.is_empty() && self.secrets.is_empty()
    }

    /// The preview in lines: the counts, then what is left out, then the verdict.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "commit preview on {}: {} staged, {} not staged, {} untracked",
            self.branch,
            self.staged.len(),
            self.not_staged.len(),
            self.untracked.len()
        )];
        if !self.staged.is_empty() {
            lines.push(format!("staged: {}", self.staged.join(", ")));
        }
        for path in &self.dirty {
            lines.push(format!(
                "warning: {path} has unsaved changes; the staged copy may differ from what is on \
                 screen"
            ));
        }
        if !self.not_staged.is_empty() {
            lines.push(format!(
                "not staged (left out): {}",
                self.not_staged.join(", ")
            ));
        }
        if !self.untracked.is_empty() {
            lines.push(format!(
                "untracked (left out): {}",
                self.untracked.join(", ")
            ));
        }
        if !self.unscanned.is_empty() {
            lines.push(format!(
                "not scanned (not text): {}",
                self.unscanned.join(", ")
            ));
        }
        for secret in &self.secrets {
            lines.push(format!("secret: {}", secret.render()));
        }
        if self.staged.is_empty() {
            lines.push("nothing is staged, so there is nothing to commit".to_owned());
        } else if !self.secrets.is_empty() {
            lines.push(format!(
                "refused: {} secret(s) would be committed",
                self.secrets.len()
            ));
        } else {
            lines.push(format!(
                "ready: {} staged path(s), no secret found",
                self.staged.len()
            ));
        }
        lines
    }
}

impl Repository {
    /// The index content of a path, as the bytes it holds.
    fn index_bytes(&self, path: &str) -> Result<Vec<u8>, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["show", &format!(":{path}")])
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot run git: {error}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
        }
    }

    /// What a commit would do, with `dirty` naming the open documents that have unsaved changes.
    ///
    /// Only the index is read: the staged content is what would be committed, so that is what is
    /// scanned. A staged path whose index content cannot be read as text is reported as unscanned
    /// rather than as clean.
    pub fn commit_preview(&self, dirty: &[String]) -> Result<CommitPreview, String> {
        let status = self.status()?;
        let mut preview = CommitPreview {
            branch: status.branch.clone(),
            staged: Vec::new(),
            not_staged: Vec::new(),
            untracked: Vec::new(),
            dirty: Vec::new(),
            secrets: Vec::new(),
            unscanned: Vec::new(),
        };
        for entry in &status.entries {
            if entry.is_untracked() {
                preview.untracked.push(entry.path.clone());
                continue;
            }
            if entry.index != ' ' && entry.index != '?' {
                preview.staged.push(entry.path.clone());
            }
            if entry.worktree != ' ' && entry.worktree != '?' {
                preview.not_staged.push(entry.path.clone());
            }
        }
        for path in &preview.staged {
            if dirty.iter().any(|open| open == path) {
                preview.dirty.push(path.clone());
            }
        }
        for path in &preview.staged {
            // A rename row is `old -> new`; the content that goes in is the new path's.
            let target = match path.rsplit_once(" -> ") {
                Some((_, target)) => target,
                None => path.as_str(),
            };
            let content = match self.index_bytes(target) {
                Ok(bytes) => bytes,
                // A deletion has no index content to scan; a path Git will not resolve is also not
                // text we can read, and both are reported rather than called clean.
                Err(_) => {
                    preview.unscanned.push(path.clone());
                    continue;
                }
            };
            let Ok(text) = std::str::from_utf8(&content) else {
                preview.unscanned.push(path.clone());
                continue;
            };
            preview.secrets.extend(secrets::scan(target, text));
        }
        Ok(preview)
    }
}

/// Splits a unified diff into its hunks, each keeping the lines Git printed.
fn hunks_of(text: &str) -> Vec<String> {
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            current = Some(format!("{line}\n"));
            continue;
        }
        if let Some(hunk) = current.as_mut() {
            hunk.push_str(line);
            hunk.push('\n');
        }
    }
    if let Some(hunk) = current {
        hunks.push(hunk);
    }
    hunks
}

#[cfg(test)]
mod tests {
    use super::{ConflictKind, Discover, HunkChoice, Repository};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn temporary(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rustage-vcs-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        path
    }

    fn git(directory: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {arguments:?} succeeds");
    }

    fn repository() -> Option<(PathBuf, Repository)> {
        if !git_available() {
            return None;
        }
        let directory = temporary("repo");
        git(&directory, &["init", "-q", "--template="]);
        git(
            &directory,
            &["config", "user.email", "rustage@example.invalid"],
        );
        git(&directory, &["config", "user.name", "rustage"]);
        std::fs::write(directory.join("a.txt"), "one\n").expect("a file");
        git(&directory, &["add", "a.txt"]);
        git(&directory, &["commit", "-q", "-m", "initial"]);
        let repository = Repository::discover(&directory).expect("a repository");
        Some((directory, repository))
    }

    fn git_text(directory: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .expect("git runs");
        assert!(output.status.success(), "git {arguments:?} succeeds");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn write(directory: &Path, path: &str, text: &str) {
        let target = directory.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("a parent directory");
        }
        std::fs::write(target, text).expect("a file");
    }

    /// A real repository whose `side` branch conflicts with the current branch on every given path.
    ///
    /// Each tuple is `(path, base, ours, theirs)`. The merge is expected to fail; the repository is
    /// left in the conflicted state so the tests exercise what the installed Git actually recorded.
    fn repository_with_conflicts(
        name: &str,
        files: &[(&str, &str, &str, &str)],
    ) -> Option<(PathBuf, Repository)> {
        if !git_available() {
            return None;
        }
        let directory = temporary(name);
        git(&directory, &["init", "-q", "--template="]);
        git(
            &directory,
            &["config", "user.email", "rustage@example.invalid"],
        );
        git(&directory, &["config", "user.name", "rustage"]);
        for (path, base, _, _) in files {
            write(&directory, path, base);
        }
        git(&directory, &["add", "--", "."]);
        git(&directory, &["commit", "-q", "-m", "base"]);
        let branch = git_text(&directory, &["rev-parse", "--abbrev-ref", "HEAD"]);
        assert!(!branch.is_empty(), "the current branch has a name");
        git(&directory, &["branch", "side"]);
        for (path, _, ours, _) in files {
            write(&directory, path, ours);
        }
        git(&directory, &["add", "--", "."]);
        git(&directory, &["commit", "-q", "-m", "ours"]);
        git(&directory, &["switch", "-q", "side"]);
        for (path, _, _, theirs) in files {
            write(&directory, path, theirs);
        }
        git(&directory, &["add", "--", "."]);
        git(&directory, &["commit", "-q", "-m", "theirs"]);
        git(&directory, &["switch", "-q", &branch]);
        let merge = Command::new("git")
            .arg("-C")
            .arg(&directory)
            .args(["merge", "side"])
            .output()
            .expect("git runs");
        assert!(!merge.status.success(), "the merge conflicts");
        let repository = Repository::discover(&directory).expect("a repository");
        Some((directory, repository))
    }

    #[test]
    fn a_directory_that_is_not_a_repository_says_so_and_a_missing_git_is_distinct() {
        if !git_available() {
            return;
        }
        let directory = temporary("plain");
        match Repository::discover(&directory) {
            Err(Discover::NotRepository(reason)) => assert!(!reason.is_empty()),
            other => panic!("expected NotRepository, got {other:?}"),
        }
    }

    #[test]
    fn status_reads_the_branch_and_a_new_path() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let clean = repository.status().expect("a status");
        assert!(clean.is_clean(), "{clean:?}");
        assert!(!clean.branch.is_empty());
        std::fs::write(directory.join("b.txt"), "two\n").expect("a file");
        let dirty = repository.status().expect("a status");
        assert!(!dirty.is_clean());
        assert!(dirty.entries.iter().any(|entry| entry.path == "b.txt"));
        assert!(dirty.entries.iter().any(|entry| entry.is_untracked()));
    }

    #[test]
    fn stage_commit_and_diff_go_through_git() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        std::fs::write(directory.join("a.txt"), "one\ntwo\n").expect("an edit");
        let diff = repository.diff(Some("a.txt")).expect("a diff");
        assert!(diff.contains("+two"), "{diff}");
        repository.stage("a.txt").expect("a stage");
        assert!(
            repository
                .staged_diff(Some("a.txt"))
                .expect("a staged diff")
                .contains("+two")
        );
        let message = repository.commit("second").expect("a commit");
        assert!(
            message.contains("second") || message.contains("1 file"),
            "{message}"
        );
        assert!(repository.status().expect("a status").is_clean());
    }

    #[test]
    fn a_dirty_tree_refuses_a_branch_switch_and_names_the_change() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        repository.create_branch("feature").expect("a clean branch");
        assert!(
            repository
                .branches()
                .expect("branches")
                .contains(&"feature".to_owned())
        );
        std::fs::write(directory.join("a.txt"), "changed\n").expect("an edit");
        let reason = repository
            .switch_branch("master")
            .or_else(|_| repository.switch_branch("main"))
            .expect_err("a dirty switch");
        assert!(reason.contains("clean worktree"), "{reason}");
        assert!(reason.contains("1 path"), "{reason}");
    }

    #[test]
    fn a_stashing_switch_keeps_the_work_and_lands_on_the_branch() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let base = repository.status().expect("a status").branch;
        repository.create_branch("feature").expect("a branch");
        std::fs::write(directory.join("a.txt"), "feature\n").expect("an edit");
        repository.stage("a.txt").expect("a stage");
        repository.commit("feature work").expect("a commit");
        repository.switch_branch(&base).expect("back to base");
        std::fs::write(directory.join("a.txt"), "unsaved work\n").expect("an edit");

        let switched = repository
            .switch_branch_stashing("feature")
            .expect("a switch");
        assert!(switched.stash.is_some(), "{switched:?}");
        assert_eq!(repository.status().expect("a status").branch, "feature");
        assert!(repository.status().expect("a status").is_clean());
        assert_eq!(
            std::fs::read_to_string(directory.join("a.txt")).expect("the file"),
            "feature\n"
        );
        let stash = repository.stash_list().expect("a list").join("\n");
        assert!(stash.contains("rustage: switch to feature"), "{stash}");
    }

    #[test]
    fn a_switch_that_fails_after_stashing_restores_the_work() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        std::fs::write(directory.join("a.txt"), "unsaved work\n").expect("an edit");
        let reason = repository
            .switch_branch_stashing("no-such-branch")
            .expect_err("a refused switch");
        assert!(reason.contains("restored"), "{reason}");
        assert_eq!(
            std::fs::read_to_string(directory.join("a.txt")).expect("the file"),
            "unsaved work\n"
        );
        assert!(!repository.status().expect("a status").is_clean());
    }

    #[test]
    fn a_clean_tree_switches_without_a_stash() {
        let Some((_directory, repository)) = repository() else {
            return;
        };
        repository.create_branch("feature").expect("a branch");
        let base = repository
            .branches()
            .expect("branches")
            .into_iter()
            .find(|name| name != "feature")
            .expect("the base branch");
        let switched = repository.switch_branch_stashing(&base).expect("a switch");
        assert_eq!(switched.stash, None, "{switched:?}");
    }

    #[test]
    fn stash_round_trips_a_change() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        std::fs::write(directory.join("a.txt"), "stashed\n").expect("an edit");
        repository.stash("work in progress").expect("a stash");
        assert!(repository.status().expect("a status").is_clean());
        assert!(!repository.stash_list().expect("a list").is_empty());
        repository.stash_pop(0).expect("a pop");
        assert!(!repository.status().expect("a status").is_clean());
        let _ = directory;
    }

    #[test]
    fn an_empty_commit_message_is_refused_before_git_runs() {
        let Some((_directory, repository)) = repository() else {
            return;
        };
        assert!(repository.commit("   ").is_err());
    }

    #[test]
    fn a_real_conflict_exposes_all_three_stages_and_a_staged_resolution() {
        let Some((directory, repository)) = repository_with_conflicts(
            "stages",
            &[
                ("a.txt", "base a\n", "ours a\n", "theirs a\n"),
                ("b.txt", "base b\n", "ours b\n", "theirs b\n"),
            ],
        ) else {
            return;
        };

        let conflicts = repository.conflicts().expect("conflicts");
        assert_eq!(conflicts.len(), 2, "{conflicts:?}");
        let a = conflicts
            .iter()
            .find(|conflict| conflict.path == "a.txt")
            .expect("a.txt is conflicted");
        assert_eq!(a.base.as_deref(), Some("base a\n"));
        assert_eq!(a.ours.as_deref(), Some("ours a\n"));
        assert_eq!(a.theirs.as_deref(), Some("theirs a\n"));
        assert_eq!(a.kind(), ConflictKind::Text);

        let text = a.text_conflict().expect("a text conflict");
        assert_eq!(text.hunk_count(), 1);
        assert_eq!(
            text.render(&[HunkChoice::Ours]).expect("a side"),
            "ours a\n"
        );

        let resolution = repository
            .resolve_text(&text, &[HunkChoice::Ours], None)
            .expect("a resolution");
        assert!(resolution.staged);
        assert!(
            !resolution.is_complete(),
            "without regeneration the resolution is not complete"
        );
        assert!(
            resolution
                .render()
                .iter()
                .any(|line| line.contains("not checked")),
            "{resolution:?}"
        );
        assert_eq!(
            std::fs::read_to_string(directory.join("a.txt")).expect("a file"),
            "ours a\n"
        );
        assert!(
            repository
                .status()
                .expect("a status")
                .entries
                .iter()
                .all(|entry| entry.path != "a.txt"),
            "resolving to the checked-out side leaves nothing unstaged for a.txt"
        );
        assert!(
            !repository
                .conflicts()
                .expect("conflicts")
                .iter()
                .any(|conflict| conflict.path == "a.txt")
        );

        let completion = repository.completion().expect("a completion");
        assert!(!completion.is_ready());
        assert_eq!(completion.unresolved, vec!["b.txt".to_owned()]);

        // The last path resolves, but a failing regeneration is not success.
        let b = repository.conflict("b.txt").expect("b.txt is conflicted");
        let text = b.text_conflict().expect("a text conflict");
        let mut regenerate = || -> Result<String, String> {
            Err("the generator refused the resolved model".to_owned())
        };
        let resolution = repository
            .resolve_text(&text, &[HunkChoice::Theirs], Some(&mut regenerate))
            .expect("a written resolution");
        assert!(resolution.staged);
        assert!(!resolution.is_complete());
        assert!(
            resolution
                .render()
                .iter()
                .any(|line| line.contains("regeneration failed")),
            "{resolution:?}"
        );
        assert!(
            repository
                .staged_diff(Some("b.txt"))
                .expect("a staged diff")
                .contains("+theirs b"),
            "the resolved bytes are in the index"
        );
        assert!(repository.completion().expect("a completion").is_ready());
    }

    #[test]
    fn a_line_selection_keeps_only_the_named_lines() {
        let Some((_directory, repository)) =
            repository_with_conflicts("lines", &[("a.txt", "a\nb\nc\n", "A\nB\nc\n", "a\nX\nY\n")])
        else {
            return;
        };
        let conflict = repository.conflict("a.txt").expect("a conflict");
        let text = conflict.text_conflict().expect("a text conflict");
        assert_eq!(text.hunk_count(), 1, "{text:?}");
        let resolved = text
            .render(&[HunkChoice::Lines {
                ours: vec![0],
                theirs: vec![2],
            }])
            .expect("a line selection");
        assert_eq!(resolved, "A\nY\n");
        assert!(text.render(&[HunkChoice::Ours]).is_ok());
        assert!(text.render(&[]).is_err(), "one choice is not enough");
        assert!(
            text.render(&[HunkChoice::Lines {
                ours: vec![9],
                theirs: Vec::new(),
            }])
            .is_err(),
            "an index outside the hunk is refused"
        );
    }

    #[test]
    fn a_generated_file_refuses_a_text_resolution_inside_a_hand_written_region() {
        const MARKER: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n";
        let base = format!(
            "{MARKER}pub fn build_ui() {{}}\n// region:handwritten save\nfn save() {{ base }}\n// endregion:handwritten save\n"
        );
        let ours = format!(
            "{MARKER}pub fn build_ui() {{}}\n// region:handwritten save\nfn save() {{ ours }}\n// endregion:handwritten save\n"
        );
        let theirs = format!(
            "{MARKER}pub fn build_ui() {{}}\n// region:handwritten save\nfn save() {{ theirs }}\n// endregion:handwritten save\n"
        );
        let Some((_directory, repository)) =
            repository_with_conflicts("generated-boundary", &[("ui.rs", &base, &ours, &theirs)])
        else {
            return;
        };
        let conflict = repository.conflict("ui.rs").expect("a conflict");
        let text = conflict.text_conflict().expect("a text conflict");
        let reason = repository
            .resolve_text(&text, &vec![HunkChoice::Ours; text.hunk_count()], None)
            .expect_err("a hand-written boundary blocks a text resolution");
        assert!(reason.contains("hand-written"), "{reason}");
        assert!(
            repository.conflict("ui.rs").is_ok(),
            "a refused resolution leaves the conflict in place"
        );
    }

    #[test]
    fn a_generated_zone_conflict_resolves_when_the_hand_written_body_is_unchanged() {
        const MARKER: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n";
        let base = format!(
            "{MARKER}pub fn build_ui() {{ let a = 1; }}\n// region:handwritten save\nfn save() {{ mine }}\n// endregion:handwritten save\n"
        );
        let ours = format!(
            "{MARKER}pub fn build_ui() {{ let a = 2; }}\n// region:handwritten save\nfn save() {{ mine }}\n// endregion:handwritten save\n"
        );
        let theirs = format!(
            "{MARKER}pub fn build_ui() {{ let a = 3; }}\n// region:handwritten save\nfn save() {{ mine }}\n// endregion:handwritten save\n"
        );
        let Some((directory, repository)) =
            repository_with_conflicts("generated-zone", &[("ui.rs", &base, &ours, &theirs)])
        else {
            return;
        };
        let conflict = repository.conflict("ui.rs").expect("a conflict");
        let text = conflict.text_conflict().expect("a text conflict");
        let resolved = text
            .render(&vec![HunkChoice::Ours; text.hunk_count()])
            .expect("a generated-zone resolution");
        assert!(resolved.contains("let a = 2;"), "{resolved}");
        let mut regenerate = || -> Result<String, String> { Ok("regenerated ui.rs".to_owned()) };
        let resolution = repository
            .resolve_text(
                &text,
                &vec![HunkChoice::Ours; text.hunk_count()],
                Some(&mut regenerate),
            )
            .expect("a resolution");
        assert!(resolution.is_complete(), "{resolution:?}");
        assert!(
            resolution
                .render()
                .iter()
                .any(|line| line.contains("regeneration: passed")),
            "{resolution:?}"
        );
        assert_eq!(
            std::fs::read_to_string(directory.join("ui.rs")).expect("a file"),
            resolved
        );
    }

    #[test]
    fn a_json_conflict_presents_keys_and_needs_an_explicit_document() {
        let Some((directory, repository)) = repository_with_conflicts(
            "structured",
            &[(
                "design.json",
                "{\"a\":1,\"b\":2}\n",
                "{\"a\":10,\"b\":2}\n",
                "{\"a\":1,\"b\":20}\n",
            )],
        ) else {
            return;
        };
        let conflict = repository.conflict("design.json").expect("a conflict");
        assert_eq!(conflict.kind(), ConflictKind::Json);
        let diff = conflict.structured_diff().expect("a structured diff");
        assert!(diff.parsed, "{diff:?}");
        assert_eq!(diff.rows.len(), 2, "{diff:?}");
        assert!(
            diff.rows
                .iter()
                .any(|row| row.key == "a" && row.ours.as_deref() == Some("10")),
            "{diff:?}"
        );

        let reason = repository
            .resolve_document("design.json", "{not json", None)
            .expect_err("a malformed document is refused");
        assert!(reason.contains("not valid JSON"), "{reason}");
        assert!(repository.conflict("design.json").is_ok());

        let resolution = repository
            .resolve_document("design.json", "{\"a\":15,\"b\":25}\n", None)
            .expect("an explicit document");
        assert!(resolution.staged);
        assert_eq!(
            std::fs::read_to_string(directory.join("design.json")).expect("a file"),
            "{\"a\":15,\"b\":25}\n"
        );
        assert!(repository.conflicts().expect("conflicts").is_empty());
    }

    #[test]
    fn abort_restores_the_state_that_was_checked_out_before_the_merge() {
        let Some((directory, repository)) =
            repository_with_conflicts("abort", &[("a.txt", "base\n", "ours\n", "theirs\n")])
        else {
            return;
        };
        assert!(!repository.completion().expect("a completion").is_ready());
        repository.merge_abort().expect("an abort");
        assert!(repository.conflicts().expect("conflicts").is_empty());
        assert!(repository.status().expect("a status").is_clean());
        assert_eq!(
            std::fs::read_to_string(directory.join("a.txt")).expect("a file"),
            "ours\n",
            "abort restores the pre-merge checkout, not the merge base"
        );
    }

    #[test]
    fn the_history_names_each_commit_and_its_time() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let history = repository.log(Some(5)).expect("a history");
        assert_eq!(history.len(), 1, "{history:?}");
        assert_eq!(history[0].subject, "initial");
        assert_eq!(history[0].author, "rustage");
        // The time is a fact; the written form is the interface's (T24.03).
        assert!(history[0].time > 1_600_000_000, "{}", history[0].time);
        assert_eq!(history[0].short().len(), 7);
        assert!(repository.log(Some(0)).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_commit_preview_reports_staged_left_out_dirty_and_secrets() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        write(&directory, "a.txt", "two\n");
        git(&directory, &["add", "a.txt"]);
        // A further edit after staging: the index holds `two`, the worktree holds `three`.
        write(&directory, "a.txt", "three\n");
        // An untracked file: it stays out too.
        write(&directory, "b.txt", "b\n");
        // A staged file that holds a secret.
        write(
            &directory,
            "c.txt",
            "api_key = a8f5f167f44f4964e6c998dee827110c\n",
        );
        git(&directory, &["add", "c.txt"]);

        let preview = repository
            .commit_preview(&["a.txt".to_owned()])
            .expect("a preview");
        assert_eq!(preview.staged, vec!["a.txt", "c.txt"]);
        assert_eq!(preview.not_staged, vec!["a.txt"]);
        assert_eq!(preview.untracked, vec!["b.txt"]);
        assert_eq!(preview.dirty, vec!["a.txt"]);
        assert_eq!(preview.secrets.len(), 1, "{preview:?}");
        assert_eq!(preview.secrets[0].path, "c.txt");
        assert_eq!(preview.secrets[0].line, 1);
        assert!(!preview.can_commit());

        let report = preview.render().join("\n");
        assert!(report.contains("has unsaved changes"), "{report}");
        assert!(report.contains("not staged (left out): a.txt"), "{report}");
        assert!(report.contains("untracked (left out): b.txt"), "{report}");
        assert!(report.contains("secret: c.txt:1"), "{report}");
        assert!(report.contains("refused: 1 secret(s)"), "{report}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_clean_staged_change_is_ready_and_binary_content_is_not_called_clean() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        write(&directory, "a.txt", "two\n");
        std::fs::write(directory.join("logo.png"), [0xffu8, 0xfe, 0x00, 0x01])
            .expect("a binary file");
        git(&directory, &["add", "a.txt", "logo.png"]);

        let preview = repository.commit_preview(&[]).expect("a preview");
        assert_eq!(preview.staged, vec!["a.txt", "logo.png"]);
        assert!(preview.secrets.is_empty(), "{preview:?}");
        assert_eq!(preview.unscanned, vec!["logo.png"]);
        assert!(preview.can_commit());
        let report = preview.render().join("\n");
        assert!(
            report.contains("not scanned (not text): logo.png"),
            "{report}"
        );
        assert!(report.contains("ready: 2 staged path(s)"), "{report}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_preview_with_nothing_staged_cannot_commit() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        write(&directory, "a.txt", "changed\n");
        let preview = repository.commit_preview(&[]).expect("a preview");
        assert!(preview.staged.is_empty());
        assert!(!preview.can_commit());
        assert!(preview.render().join("\n").contains("nothing is staged"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn one_hunk_can_be_staged_while_the_rest_stay_in_the_worktree() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        // Three changes far enough apart that Git prints three hunks.
        let baseline: Vec<String> = (1..=30).map(|number| format!("line {number}\n")).collect();
        write(&directory, "a.txt", &baseline.concat());
        git(&directory, &["add", "a.txt"]);
        git(&directory, &["commit", "-q", "-m", "baseline"]);
        let mut lines = baseline;
        lines[1] = "line 2 changed\n".to_owned();
        lines[14] = "line 15 changed\n".to_owned();
        lines[28] = "line 29 changed\n".to_owned();
        let text: String = lines.concat();
        write(&directory, "a.txt", &text);

        let hunks = repository.diff_hunks("a.txt").expect("hunks");
        assert_eq!(hunks.len(), 3, "{hunks:?}");
        assert!(hunks[0].starts_with("@@"), "{hunks:?}");

        let staged = repository.stage_hunk("a.txt", 1).expect("a staged hunk");
        assert!(staged.contains("staged hunk 1 of a.txt"), "{staged}");
        let index = git_text(&directory, &["diff", "--cached", "-U0", "--", "a.txt"]);
        assert!(index.contains("line 2 changed"), "{index}");
        assert!(!index.contains("line 15 changed"), "{index}");
        assert!(!index.contains("line 29 changed"), "{index}");

        // The working copy still holds all three edits.
        let worktree = std::fs::read_to_string(directory.join("a.txt")).expect("a file");
        assert!(worktree.contains("line 15 changed"));
        assert!(worktree.contains("line 29 changed"));

        // A hunk number outside the printed range is refused rather than applied to another one.
        assert!(repository.stage_hunk("a.txt", 0).is_err());
        assert!(repository.stage_hunk("a.txt", 4).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_binary_change_has_no_hunks_to_stage() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        std::fs::write(directory.join("logo.png"), [0xffu8, 0xfe, 0x00]).expect("a binary file");
        git(&directory, &["add", "logo.png"]);
        git(&directory, &["commit", "-q", "-m", "binary"]);
        std::fs::write(directory.join("logo.png"), [0xffu8, 0xfe, 0x01]).expect("a binary change");

        assert!(repository.diff_hunks("logo.png").expect("hunks").is_empty());
        let error = repository
            .stage_hunk("logo.png", 1)
            .expect_err("nothing to stage");
        assert!(error.contains("no unstaged text change"), "{error}");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_commit_shows_its_diff_and_a_paths_content_at_that_revision() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        write(&directory, "a.txt", "two\n");
        git(&directory, &["add", "a.txt"]);
        git(&directory, &["commit", "-q", "-m", "second"]);

        let diff = repository.commit_show("HEAD", None).expect("a diff");
        assert!(diff.contains("second"), "{diff}");
        assert!(diff.contains("-one"), "{diff}");
        assert!(diff.contains("+two"), "{diff}");

        let content = repository
            .commit_show("HEAD", Some("a.txt"))
            .expect("a file at a revision");
        assert_eq!(content, "two\n");
        assert!(repository.commit_show("", None).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn restoring_a_path_is_previewed_and_nothing_is_written() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        write(&directory, "a.txt", "changed\n");
        let preview = repository
            .restore_preview("HEAD", "a.txt")
            .expect("a preview");
        assert!(preview.differs);
        assert!(!preview.missing);
        assert_eq!(preview.bytes, "one\n".len());
        let lines = preview.render().join("\n");
        assert!(lines.contains("the working copy differs"), "{lines}");
        assert!(lines.contains("nothing was written"), "{lines}");
        // The preview is a preview: the working copy still holds the change.
        assert_eq!(
            std::fs::read_to_string(directory.join("a.txt")).expect("a file"),
            "changed\n"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_identical_working_copy_and_a_missing_one_are_told_apart() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        let same = repository
            .restore_preview("HEAD", "a.txt")
            .expect("a preview");
        assert!(!same.differs && !same.missing);
        assert!(same.render()[1].contains("identical"));

        std::fs::remove_file(directory.join("a.txt")).expect("a remove");
        let gone = repository
            .restore_preview("HEAD", "a.txt")
            .expect("a preview");
        assert!(gone.differs && gone.missing);
        assert!(gone.render()[1].contains("no such file"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_path_absent_from_the_revision_is_refused_rather_than_previewed_empty() {
        let Some((directory, repository)) = repository() else {
            return;
        };
        assert!(repository.restore_preview("HEAD", "ghost.txt").is_err());
        assert!(repository.restore_preview("", "a.txt").is_err());
        assert!(repository.restore_preview("HEAD", "").is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
