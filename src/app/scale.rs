// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! How the editor scales to a large document (task T06.07).
//!
//! The acceptance line is *"the size boundaries and every degradation are visible"*. A reader must be
//! able to ask **why** a feature is off, so this module gives every answer a reason rather than a bare
//! `false`:
//!
//! * **A document is classified once, into a tier.** [`Scale::tier`] is [`Normal`](Tier::Normal),
//!   [`Large`](Tier::Large) or [`Huge`](Tier::Huge), decided from the document's byte size against
//!   [`Limits`]. The boundaries are numbers in one place, not scattered `if`s.
//! * **A feature is allowed or refused with a reason.** [`Scale::allows`] answers yes or no and
//!   [`Scale::reason`] says why, naming the tier or the very long line. A feature that was turned off
//!   can always be explained.
//! * **One very long line degrades the per-character features.** A minified file is a few huge lines,
//!   so highlighting and wrapping are off with a reason naming the line's column count, while the
//!   document is not called "huge" for its byte size alone.
//! * **The highlight cache has a ceiling.** [`Scale::cache_cap`] is how many lines the cache may keep;
//!   [`Scale::trims_cache`] says the document is longer than that, so a reader knows some lines are
//!   re-lexed rather than kept.
//! * **Opening is cancellable.** [`Cancellation`] is a token a worker checks between chunks, and
//!   [`read_cancellable`] reads a file only while the token is live, dropping what it read on a
//!   cancellation rather than returning a partial document.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A feature the editor can turn off for a large document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    /// Syntax highlighting.
    Highlight,
    /// Fold regions.
    Folding,
    /// The minimap.
    Minimap,
    /// Bracket matching.
    BracketMatching,
    /// Wrapping long lines to the viewport.
    Wrapping,
}

impl Feature {
    /// Every feature, in a stable order.
    pub const ALL: [Self; 5] = [
        Self::Highlight,
        Self::Folding,
        Self::Minimap,
        Self::BracketMatching,
        Self::Wrapping,
    ];

    /// The feature's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Highlight => "highlight",
            Self::Folding => "folding",
            Self::Minimap => "minimap",
            Self::BracketMatching => "bracket-matching",
            Self::Wrapping => "wrapping",
        }
    }

    /// The feature a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|feature| feature.name() == name)
    }
}

/// How heavy a document is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Tier {
    /// Below the large boundary: everything is on.
    Normal,
    /// At or above the large boundary, below the huge one.
    Large,
    /// At or above the huge boundary: plain text.
    Huge,
}

impl Tier {
    /// The tier's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::Huge => "huge",
        }
    }
}

/// Where the size boundaries are, in one place.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    /// At or above this many bytes, the document is [`Tier::Large`].
    pub large_bytes: usize,
    /// At or above this many bytes, the document is [`Tier::Huge`].
    pub huge_bytes: usize,
    /// A line longer than this many columns degrades the per-character features.
    pub long_line: usize,
    /// How many lines the highlight cache may keep.
    pub cache_lines: usize,
}

impl Limits {
    /// A one-mebibyte large boundary, an eight-mebibyte huge one, a four-thousand-column long line and
    /// a twenty-thousand-line cache ceiling.
    pub const fn new() -> Self {
        Self {
            large_bytes: 1 << 20,
            huge_bytes: 8 << 20,
            long_line: 4_000,
            cache_lines: 20_000,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new()
    }
}

/// A document's size, classified, with the decisions that follow from it.
#[derive(Clone, Debug)]
pub struct Scale {
    limits: Limits,
    bytes: usize,
    lines: usize,
    longest_line: usize,
    tier: Tier,
}

impl Scale {
    /// Classifies a text.
    pub fn of(text: &str, limits: Limits) -> Self {
        let bytes = text.len();
        let mut lines = 1usize;
        let mut longest_line = 0usize;
        let mut current = 0usize;
        for character in text.chars() {
            if character == '\n' {
                lines += 1;
                longest_line = longest_line.max(current);
                current = 0;
            } else {
                current += 1;
            }
        }
        longest_line = longest_line.max(current);
        let tier = if bytes >= limits.huge_bytes {
            Tier::Huge
        } else if bytes >= limits.large_bytes {
            Tier::Large
        } else {
            Tier::Normal
        };
        Self {
            limits,
            bytes,
            lines,
            longest_line,
            tier,
        }
    }

    /// The boundaries this classification used.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The tier.
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// How many bytes the document is.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// How many lines it has.
    pub fn lines(&self) -> usize {
        self.lines
    }

    /// How many columns its longest line is.
    pub fn longest_line(&self) -> usize {
        self.longest_line
    }

    /// Whether the document has a line past the long-line boundary.
    pub fn has_long_line(&self) -> bool {
        self.longest_line > self.limits.long_line
    }

    /// How many lines the highlight cache may keep for this document.
    pub fn cache_cap(&self) -> usize {
        self.lines.min(self.limits.cache_lines)
    }

    /// Whether the document is longer than the cache ceiling.
    pub fn trims_cache(&self) -> bool {
        self.lines > self.limits.cache_lines
    }

    /// Why a feature is off, or `None` when it is on.
    pub fn reason(&self, feature: Feature) -> Option<String> {
        if self.has_long_line() {
            match feature {
                Feature::Highlight => {
                    return Some(format!(
                        "the document has a line of {} columns; highlighting a very long line is off",
                        self.longest_line
                    ));
                }
                Feature::Wrapping => {
                    return Some(format!(
                        "the document has a line of {} columns; wrapping a very long line is off",
                        self.longest_line
                    ));
                }
                _ => {}
            }
        }
        match self.tier {
            Tier::Normal => None,
            Tier::Large => match feature {
                Feature::Folding => Some("the document is large; folding is off".to_owned()),
                Feature::Minimap => Some("the document is large; the minimap is off".to_owned()),
                _ => None,
            },
            Tier::Huge => Some(format!(
                "the document is very large; {} is off",
                feature.name()
            )),
        }
    }

    /// Whether a feature is on.
    pub fn allows(&self, feature: Feature) -> bool {
        self.reason(feature).is_none()
    }

    /// The classification in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "scale: {} byte(s), {} line(s), longest {} column(s), tier {}",
                self.bytes,
                self.lines,
                self.longest_line,
                self.tier.name()
            ),
            format!(
                "cache: {} line(s) kept of {}, {}",
                self.cache_cap(),
                self.limits.cache_lines,
                if self.trims_cache() {
                    "trimmed"
                } else {
                    "complete"
                }
            ),
        ];
        for feature in Feature::ALL {
            lines.push(match self.reason(feature) {
                None => format!("  {}: on", feature.name()),
                Some(reason) => format!("  {}: off — {reason}", feature.name()),
            });
        }
        lines
    }
}

/// A token a worker checks between chunks of a long operation.
#[derive(Clone, Debug, Default)]
pub struct Cancellation {
    flag: Arc<AtomicBool>,
}

impl Cancellation {
    /// A live token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the operation the token belongs to.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether it has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Refuses when it has been cancelled.
    pub fn check(&self) -> Result<(), OpenError> {
        if self.is_cancelled() {
            Err(OpenError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Why an open did not finish.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum OpenError {
    /// The file could not be read.
    Io(String),
    /// The open was cancelled, and nothing was returned.
    Cancelled,
}

impl OpenError {
    /// The reason, as one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::Io(reason) => format!("cannot read the file: {reason}"),
            Self::Cancelled => "opening was cancelled".to_owned(),
        }
    }
}

/// Reads a whole file, checking the token between chunks.
///
/// The read happens in `chunk`-byte pieces, and the token is checked before each one. A cancellation
/// discards everything read so far and returns [`OpenError::Cancelled`], so a cancelled open never
/// hands back a partial document. The default chunk is 64 KiB.
pub fn read_cancellable(
    path: &Path,
    cancellation: &Cancellation,
    chunk: usize,
) -> Result<Vec<u8>, OpenError> {
    use std::io::Read;

    cancellation.check()?;
    let mut file = std::fs::File::open(path).map_err(|error| OpenError::Io(error.to_string()))?;
    let mut out = Vec::new();
    let mut buffer = vec![0u8; chunk.max(1)];
    loop {
        cancellation.check()?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| OpenError::Io(error.to_string()))?;
        if read == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buffer[..read]);
    }
}

#[cfg(test)]
mod tests {
    use super::{Cancellation, Feature, Limits, OpenError, Scale, Tier, read_cancellable};

    fn limits() -> Limits {
        Limits {
            large_bytes: 100,
            huge_bytes: 1_000,
            long_line: 10,
            cache_lines: 5,
        }
    }

    #[test]
    fn a_small_document_keeps_every_feature_on() {
        let scale = Scale::of("one\ntwo\n", limits());
        assert_eq!(scale.tier(), Tier::Normal);
        assert_eq!(scale.lines(), 3);
        assert_eq!(scale.longest_line(), 3);
        for feature in Feature::ALL {
            assert!(scale.allows(feature), "{} should be on", feature.name());
            assert!(scale.reason(feature).is_none());
        }
    }

    #[test]
    fn a_large_document_turns_folding_and_the_minimap_off_with_a_reason() {
        // Many short lines, so it is large by size and not by having one long line.
        let scale = Scale::of(&"aaaaaaaaaa\n".repeat(20), limits());
        assert_eq!(scale.tier(), Tier::Large);
        assert!(scale.allows(Feature::Highlight));
        assert!(scale.allows(Feature::BracketMatching));
        assert!(!scale.allows(Feature::Folding));
        assert!(!scale.allows(Feature::Minimap));
        assert!(scale.reason(Feature::Folding).unwrap().contains("large"));
    }

    #[test]
    fn a_huge_document_is_plain_text() {
        let scale = Scale::of(&"aaaaaaaaaa\n".repeat(200), limits());
        assert_eq!(scale.tier(), Tier::Huge);
        for feature in Feature::ALL {
            assert!(!scale.allows(feature), "{} should be off", feature.name());
            assert!(scale.reason(feature).is_some());
        }
    }

    #[test]
    fn one_very_long_line_degrades_highlighting_and_wrapping_on_its_own() {
        // A small document, but with a line past the long-line boundary.
        let scale = Scale::of("x\nxxxxxxxxxxxxxxxxxxxx\ny\n", limits());
        assert_eq!(scale.tier(), Tier::Normal, "it is not large by size");
        assert!(scale.has_long_line());
        assert!(!scale.allows(Feature::Highlight));
        assert!(!scale.allows(Feature::Wrapping));
        assert!(
            scale
                .reason(Feature::Highlight)
                .unwrap()
                .contains("20 column")
        );
        // Folding and the minimap are still on: the document is not large.
        assert!(scale.allows(Feature::Folding));
        assert!(scale.allows(Feature::Minimap));
    }

    #[test]
    fn the_cache_ceiling_is_reported_when_the_document_is_longer() {
        let scale = Scale::of("a\nb\nc\nd\ne\nf\ng\n", limits());
        assert_eq!(scale.lines(), 8);
        assert_eq!(scale.cache_cap(), 5);
        assert!(scale.trims_cache());
        let short = Scale::of("a\nb\n", limits());
        assert_eq!(short.cache_cap(), 3);
        assert!(!short.trims_cache());
    }

    #[test]
    fn a_cancelled_open_returns_nothing() {
        let path = std::env::temp_dir().join("rustage-scale-cancel.txt");
        std::fs::write(&path, b"one\ntwo\nthree\n").unwrap();

        // A live token reads the whole file.
        let token = Cancellation::new();
        let bytes = read_cancellable(&path, &token, 4).expect("a live read finishes");
        assert_eq!(bytes, b"one\ntwo\nthree\n");

        // A cancelled token refuses before reading anything, and returns no partial document.
        let token = Cancellation::new();
        token.cancel();
        match read_cancellable(&path, &token, 4) {
            Err(OpenError::Cancelled) => {}
            other => panic!("expected a cancellation, got {other:?}"),
        }

        std::fs::remove_file(&path).ok();
    }
}
