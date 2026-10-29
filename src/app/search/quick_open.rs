// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Quick open, file and workspace symbols, ranking and recent access (task T08.03).
//!
//! The acceptance line is *"file search still works when the service is missing"*. That is a statement
//! about a **dependency**, so the module keeps the two paths apart rather than assuming the service is
//! always there:
//!
//! * **File search reads the file list of [T08.01](crate::project::record::index), not the service.** A
//!   [`QuickOpen`] is built from candidate paths and matched with [`fuzzy_score`]; nothing on that path
//!   consults a language service, so it cannot fail because one is missing.
//! * **Symbol search needs a negotiated capability, and says so.** [`symbols`] answers
//!   [`SymbolRefusal`] when [`Capability::Symbols`] was not negotiated, so the *symbols*
//!   half of quick open is the only half that is disabled — and it is disabled explicitly, not by
//!   returning nothing.
//! * **Ranking is match quality first, recency second.** [`Hit`] carries both; a better match beats a
//!   more recent one, so a person's muscle memory does not win over the thing they typed, while
//!   [`QuickOpen::touch`] breaks ties toward what was opened last.

#![allow(dead_code)]

use crate::app::language::session::{Capabilities, Capability};

/// A file quick open can offer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Candidate {
    /// The path.
    pub path: String,
    /// How recently it was opened; higher is more recent.
    pub recent: u64,
}

impl Candidate {
    /// A candidate that has never been opened.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            recent: 0,
        }
    }
}

/// One result of quick open.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    /// The label shown.
    pub label: String,
    /// The path, for a file.
    pub path: Option<String>,
    /// A score: higher is a better match.
    pub score: i64,
    /// How recently it was opened.
    pub recent: u64,
}

impl Hit {
    /// The hit in one line.
    pub fn render(&self) -> String {
        match &self.path {
            Some(path) => format!("{} (score {}, recent {})", path, self.score, self.recent),
            None => format!("{} (score {})", self.label, self.score),
        }
    }
}

/// The quick open over a set of files, which needs no service.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct QuickOpen {
    candidates: Vec<Candidate>,
}

impl QuickOpen {
    /// No candidates.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file.
    pub fn add(&mut self, candidate: Candidate) {
        self.candidates.push(candidate);
    }

    /// Records that a file was opened, making it more recent than anything opened before.
    pub fn touch(&mut self, path: &str) {
        let next = self
            .candidates
            .iter()
            .map(|candidate| candidate.recent)
            .max()
            .unwrap_or(0)
            + 1;
        if let Some(candidate) = self.candidates.iter_mut().find(|c| c.path == path) {
            candidate.recent = next;
        } else {
            self.candidates.push(Candidate {
                path: path.to_owned(),
                recent: next,
            });
        }
    }

    /// How many files it knows.
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    /// Whether it knows none.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// The best matches for a query, best first.
    pub fn open(&self, query: &str, limit: usize) -> Vec<Hit> {
        let mut hits: Vec<Hit> = self
            .candidates
            .iter()
            .filter_map(|candidate| {
                fuzzy_score(query, &candidate.path).map(|score| Hit {
                    label: candidate.path.clone(),
                    path: Some(candidate.path.clone()),
                    score,
                    recent: candidate.recent,
                })
            })
            .collect();
        rank(&mut hits);
        hits.truncate(limit);
        hits
    }
}

/// Scores a fuzzy match of a query against a candidate, or `None` when it does not match.
///
/// A candidate matches when every query character appears in order. The score rewards a match at the
/// start, contiguous runs and a shorter candidate, so `src/main.rs` beats `src/models/main.rs` for
/// `main`.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let candidate_chars: Vec<char> = candidate.chars().collect();
    let lowered: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut score: i64 = 0;
    let mut cursor = 0usize;
    let mut previous: Option<usize> = None;
    for (index, wanted) in query.iter().enumerate() {
        let found = (cursor..lowered.len()).find(|at| lowered[*at] == *wanted)?;
        // A contiguous run is worth more than a scattered one.
        if previous == Some(found.wrapping_sub(1)) {
            score += 8;
        }
        // A match at the start or after a separator is worth more.
        if found == 0
            || matches!(
                candidate_chars.get(found - 1),
                Some('/') | Some('_') | Some('-')
            )
        {
            score += 6;
        }
        // A shorter candidate is a better answer for the same match.
        score += 1;
        if index == 0 {
            score += 4;
        }
        previous = Some(found);
        cursor = found + 1;
    }
    // Prefer shorter paths, mildly.
    score -= (candidate.len() as i64) / 8;
    Some(score)
}

/// Orders hits: best score first, then most recent.
fn rank(hits: &mut [Hit]) {
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.recent.cmp(&a.recent))
            .then(a.label.cmp(&b.label))
    });
}

/// What kind of symbols a search asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymbolMode {
    /// Symbols in one file.
    File,
    /// Symbols across the workspace.
    Workspace,
}

impl SymbolMode {
    /// The mode's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::File => "file-symbols",
            Self::Workspace => "workspace-symbols",
        }
    }

    /// The mode a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "file-symbols" | "file" => Some(Self::File),
            "workspace-symbols" | "workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

/// A symbol quick open can offer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SymbolEntry {
    /// The symbol's name.
    pub name: String,
    /// The file it is in.
    pub path: String,
}

/// Why a symbol search is disabled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SymbolRefusal {
    /// The capability that was not negotiated.
    pub capability: Capability,
}

impl SymbolRefusal {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        format!(
            "the service did not negotiate {}, so symbol search is disabled",
            self.capability.name()
        )
    }
}

/// Searches symbols, which needs the service's `symbols` capability.
pub fn symbols(
    entries: &[SymbolEntry],
    capabilities: &Capabilities,
    query: &str,
    limit: usize,
) -> Result<Vec<Hit>, SymbolRefusal> {
    if !capabilities.supports(Capability::Symbols) {
        return Err(SymbolRefusal {
            capability: Capability::Symbols,
        });
    }
    let mut hits: Vec<Hit> = entries
        .iter()
        .filter_map(|entry| {
            fuzzy_score(query, &entry.name).map(|score| Hit {
                label: format!("{} ({})", entry.name, entry.path),
                path: Some(entry.path.clone()),
                score,
                recent: 0,
            })
        })
        .collect();
    rank(&mut hits);
    hits.truncate(limit);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::{Candidate, QuickOpen, SymbolEntry, SymbolMode, fuzzy_score, symbols};
    use crate::app::language::session::{Capabilities, Capability};

    fn quick() -> QuickOpen {
        let mut open = QuickOpen::new();
        for path in [
            "src/main.rs",
            "src/models/main.rs",
            "src/lib.rs",
            "tests/smoke.rs",
        ] {
            open.add(Candidate::new(path));
        }
        open
    }

    #[test]
    fn file_search_works_without_a_service() {
        // There is no capability anywhere here: file search is not gated on one.
        let open = quick();
        let hits = open.open("main", 10);
        assert_eq!(hits.len(), 2);
        // The shorter path wins for the same match.
        assert_eq!(hits[0].path.as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn symbol_search_is_disabled_without_the_capability() {
        let entries = vec![SymbolEntry {
            name: "main".to_owned(),
            path: "src/main.rs".to_owned(),
        }];
        let without = Capabilities::of([Capability::Completion]);
        assert!(symbols(&entries, &without, "main", 10).is_err());
        // The same file search still works, which is the acceptance.
        assert!(!quick().open("main", 10).is_empty());
        // With the capability, symbols are found.
        let with = Capabilities::of([Capability::Symbols]);
        let hits = symbols(&entries, &with, "main", 10).expect("enabled");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn recency_breaks_ties() {
        let mut open = quick();
        // Two equally good matches: make one recent.
        open.add(Candidate::new("a/x.rs"));
        open.add(Candidate::new("b/x.rs"));
        open.touch("b/x.rs");
        let hits = open.open("x.rs", 10);
        assert_eq!(hits[0].path.as_deref(), Some("b/x.rs"));
    }

    #[test]
    fn a_non_match_is_not_a_hit() {
        assert_eq!(fuzzy_score("zzz", "src/main.rs"), None);
        assert!(fuzzy_score("s", "src/main.rs").is_some());
        // Out-of-order characters do not match.
        assert_eq!(fuzzy_score("niam", "main"), None);
    }

    #[test]
    fn a_query_at_the_start_scores_higher_than_one_in_the_middle() {
        let start = fuzzy_score("main", "main.rs").expect("a match");
        let middle = fuzzy_score("main", "x/main.rs").expect("a match");
        assert!(start > middle);
    }

    #[test]
    fn the_limit_truncates() {
        let open = quick();
        assert_eq!(open.open("", 2).len(), 2);
    }

    #[test]
    fn a_mode_name_round_trips() {
        assert_eq!(
            SymbolMode::from_name("file-symbols"),
            Some(SymbolMode::File)
        );
        assert_eq!(
            SymbolMode::from_name("workspace-symbols"),
            Some(SymbolMode::Workspace)
        );
        assert_eq!(SymbolMode::from_name("nowhere"), None);
    }
}
