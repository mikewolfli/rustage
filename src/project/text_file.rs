// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A file's bytes and how they become editable text (task T06.05).
//!
//! The acceptance line is *"non-UTF-8 and mixed line endings have a policy, and nothing is silently
//! corrupted"*. A policy is a decision made once and then visible, so this module makes the three
//! decisions that a `String::from_utf8_lossy` and a blanket `replace` would otherwise hide:
//!
//! * **Undecodable bytes are refused, not replaced.** `from_utf8_lossy` turns every byte that is not
//!   UTF-8 into U+FFFD and forgets where it was; [`TextFile::open`] returns [`TextFileError::NotUtf8`]
//!   naming the offset, so a caller cannot quietly save a document that changed under it.
//! * **The byte-order mark is a fact, not noise.** A UTF-8 BOM is stripped from the text and
//!   remembered, so [`TextFile::encode`] writes it back; a UTF-16/UTF-32 BOM is refused by name rather
//!   than read as garbage.
//! * **Mixed line endings cannot be flattened by accident.** Reading normalises every ending to `\n`
//!   for editing but keeps the counts, and [`TextFile::encode`] **refuses** a file that mixed them;
//!   a caller that wants one ending asks for it explicitly with [`TextFile::encode_with`]. New lines
//!   use the dominant ending, which [`LineEndings::ending`] reports.
//!
//! Indentation is the fourth decision: whether the file indents with tabs or spaces (and how wide),
//! read once from the text so the editor, the gutter and a formatter agree on one number.

#![allow(dead_code)]

use crate::project::text::TextStore;

/// A file that could not be read as editable text, with the reason why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TextFileError {
    /// A byte that is not valid UTF-8, at this offset.
    NotUtf8 { at: usize },
    /// A byte-order mark for an encoding this build does not read.
    Unsupported { encoding: &'static str },
    /// Line endings that disagree, so writing one ending would change the file.
    MixedLineEndings { lf: usize, crlf: usize, cr: usize },
}

impl TextFileError {
    /// The reason, as one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::NotUtf8 { at } => format!("byte {at} is not UTF-8"),
            Self::Unsupported { encoding } => {
                format!("{encoding} is not supported (this build reads UTF-8)")
            }
            Self::MixedLineEndings { lf, crlf, cr } => format!(
                "the file mixes line endings ({lf} LF, {crlf} CRLF, {cr} CR); \
                 writing one of them is a choice, not a default"
            ),
        }
    }
}

/// How the bytes of a file start.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    /// Plain UTF-8, no byte-order mark.
    Utf8,
    /// UTF-8 with a leading byte-order mark, which is stripped on read and written back.
    Utf8Bom,
}

impl Encoding {
    /// The encoding, as it is written in a report.
    pub fn render(&self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8 with BOM",
        }
    }
}

/// The line ending a new line gets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ending {
    /// `\n`, the Unix ending.
    Lf,
    /// `\r\n`, the Windows ending.
    CrLf,
    /// `\r`, the classic Mac ending.
    Cr,
}

impl Ending {
    /// The ending, as it is written in a file.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }

    /// The ending, as it is written in a report.
    pub fn render(&self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::Cr => "CR",
        }
    }

    /// The ending a name picks, for a caller that chooses one on the command line.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "lf" => Some(Self::Lf),
            "crlf" => Some(Self::CrLf),
            "cr" => Some(Self::Cr),
            _ => None,
        }
    }
}

/// How many of each line ending a file holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LineEndings {
    lf: usize,
    crlf: usize,
    cr: usize,
}

impl LineEndings {
    /// Counts the endings in a text, without changing it.
    pub fn detect(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut endings = Self::default();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\r' => {
                    if bytes.get(index + 1) == Some(&b'\n') {
                        endings.crlf += 1;
                        index += 2;
                        continue;
                    }
                    endings.cr += 1;
                }
                b'\n' => endings.lf += 1,
                _ => {}
            }
            index += 1;
        }
        endings
    }

    /// How many `\n` endings.
    pub fn lf(&self) -> usize {
        self.lf
    }

    /// How many `\r\n` endings.
    pub fn crlf(&self) -> usize {
        self.crlf
    }

    /// How many `\r` endings.
    pub fn cr(&self) -> usize {
        self.cr
    }

    /// How many endings there are, of every kind.
    pub fn total(&self) -> usize {
        self.lf + self.crlf + self.cr
    }

    /// Whether more than one kind of ending is present.
    pub fn is_mixed(&self) -> bool {
        [self.lf, self.crlf, self.cr]
            .iter()
            .filter(|count| **count > 0)
            .count()
            > 1
    }

    /// The ending a new line gets: the most common one, with a tie going to `\n`.
    pub fn ending(&self) -> Ending {
        let mut best = (self.lf, Ending::Lf);
        if self.crlf > best.0 {
            best = (self.crlf, Ending::CrLf);
        }
        if self.cr > best.0 {
            best = (self.cr, Ending::Cr);
        }
        best.1
    }

    /// The endings, as they are written in a report.
    pub fn render(&self) -> String {
        let kind = if self.is_mixed() {
            "mixed".to_owned()
        } else {
            self.ending().render().to_owned()
        };
        format!(
            "{kind} ({} LF, {} CRLF, {} CR)",
            self.lf, self.crlf, self.cr
        )
    }
}

/// How a file indents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Indentation {
    /// A leading tab is one level.
    Tabs,
    /// A level is this many spaces.
    Spaces { width: u32 },
}

impl Indentation {
    /// Reads the file's indentation from its text, by a stable rule.
    ///
    /// Tabs win when at least as many lines start with a tab as with spaces, because a file that has
    /// any tab indentation is a tabs file. Otherwise the unit is the greatest common divisor of the
    /// positive space indents, which recovers `4` from a file indented by 4, 8 and 12 spaces.
    pub fn detect(text: &str) -> Self {
        let mut tab_lines = 0usize;
        let mut space_runs: Vec<usize> = Vec::new();
        for line in text.split('\n') {
            let indent: &str = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
            if indent.is_empty() {
                continue;
            }
            if indent.contains('\t') {
                tab_lines += 1;
            } else {
                space_runs.push(indent.len());
            }
        }
        if tab_lines > 0 && tab_lines >= space_runs.len() {
            return Self::Tabs;
        }
        let unit = space_runs.iter().fold(0usize, |acc, run| gcd(acc, *run));
        let width = if unit == 0 { 4 } else { unit.clamp(1, 8) };
        Self::Spaces {
            width: width as u32,
        }
    }

    /// How many columns one level is, whichever kind of indentation it is.
    pub fn width(&self, tab_width: u32) -> u32 {
        match self {
            Self::Tabs => tab_width,
            Self::Spaces { width } => *width,
        }
    }

    /// The indentation, as it is written in a report.
    pub fn render(&self) -> String {
        match self {
            Self::Tabs => "tabs".to_owned(),
            Self::Spaces { width } => format!("{width} spaces"),
        }
    }
}

/// A file's bytes, read into editable text with the decisions it was read by.
#[derive(Clone, Debug)]
pub struct TextFile {
    store: TextStore,
    encoding: Encoding,
    endings: LineEndings,
    indentation: Indentation,
}

impl TextFile {
    /// Reads a file's bytes, and refuses anything it cannot read without guessing.
    pub fn open(bytes: &[u8]) -> Result<Self, TextFileError> {
        let (body, encoding) = strip_bom(bytes)?;
        let text = std::str::from_utf8(body).map_err(|error| TextFileError::NotUtf8 {
            at: error.valid_up_to(),
        })?;
        let mut file = Self::from_text(text);
        file.encoding = encoding;
        Ok(file)
    }

    /// Makes a file from text that is already UTF-8 (a new document, or a test).
    pub fn from_text(text: &str) -> Self {
        let endings = LineEndings::detect(text);
        let text = normalise(text);
        let indentation = Indentation::detect(&text);
        Self {
            store: TextStore::new(text),
            encoding: Encoding::Utf8,
            endings,
            indentation,
        }
    }

    /// The text, with every line ending normalised to `\n`.
    pub fn text(&self) -> String {
        self.store.text()
    }

    /// The editable store, for edits.
    pub fn store(&self) -> &TextStore {
        &self.store
    }

    /// The editable store, for edits that change it.
    pub fn store_mut(&mut self) -> &mut TextStore {
        &mut self.store
    }

    /// The encoding the bytes were read with.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The line endings the bytes held.
    pub fn endings(&self) -> LineEndings {
        self.endings
    }

    /// The indentation read from the text.
    pub fn indentation(&self) -> Indentation {
        self.indentation
    }

    /// How many lines the text has.
    pub fn line_count(&self) -> usize {
        self.store.text().matches('\n').count() + 1
    }

    /// The bytes to write, or a refusal when the file mixed its line endings.
    ///
    /// A file that mixed endings has no single ending to write, so writing one would change bytes
    /// that were never meant to change. The refusal is the policy: choose an ending explicitly with
    /// [`TextFile::encode_with`].
    pub fn encode(&self) -> Result<Vec<u8>, TextFileError> {
        if self.endings.is_mixed() {
            return Err(TextFileError::MixedLineEndings {
                lf: self.endings.lf(),
                crlf: self.endings.crlf(),
                cr: self.endings.cr(),
            });
        }
        Ok(self.encode_with(self.endings.ending()))
    }

    /// The bytes to write with one ending, chosen by the caller.
    pub fn encode_with(&self, ending: Ending) -> Vec<u8> {
        let body = match ending {
            Ending::Lf => self.store.text(),
            Ending::CrLf => self.store.text().replace('\n', "\r\n"),
            Ending::Cr => self.store.text().replace('\n', "\r"),
        };
        let mut out = Vec::with_capacity(body.len() + 3);
        if self.encoding == Encoding::Utf8Bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        out.extend_from_slice(body.as_bytes());
        out
    }

    /// The file's policy in one report.
    pub fn render(&self) -> Vec<String> {
        vec![
            format!("encoding: {}", self.encoding.render()),
            format!("line endings: {}", self.endings.render()),
            format!("indentation: {}", self.indentation.render()),
            format!(
                "text: {} byte(s), {} line(s)",
                self.store.len(),
                self.line_count()
            ),
        ]
    }
}

/// Strips a leading byte-order mark, or refuses one this build does not read.
fn strip_bom(bytes: &[u8]) -> Result<(&[u8], Encoding), TextFileError> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Ok((&bytes[3..], Encoding::Utf8Bom));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Err(TextFileError::Unsupported {
            encoding: "UTF-16LE",
        });
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(TextFileError::Unsupported {
            encoding: "UTF-16BE",
        });
    }
    Ok((bytes, Encoding::Utf8))
}

/// Every line ending normalised to `\n`.
fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The greatest common divisor, with `gcd(0, n) == n`.
fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

#[cfg(test)]
mod tests {
    use super::{Encoding, Ending, Indentation, LineEndings, TextFile, TextFileError};

    #[test]
    fn a_plain_utf8_file_round_trips_byte_for_byte() {
        let bytes = b"one\ntwo\nthree\n";
        let file = TextFile::open(bytes).expect("plain UTF-8");
        assert_eq!(file.encoding(), Encoding::Utf8);
        assert_eq!(file.text(), "one\ntwo\nthree\n");
        assert_eq!(file.line_count(), 4);
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn a_bom_is_stripped_and_written_back() {
        let bytes = b"\xEF\xBB\xBFone\ntwo\n";
        let file = TextFile::open(bytes).expect("UTF-8 with a BOM");
        assert_eq!(file.encoding(), Encoding::Utf8Bom);
        // The mark is not part of the text an editor shows.
        assert_eq!(file.text(), "one\ntwo\n");
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn a_non_utf8_byte_is_refused_with_its_offset_not_replaced() {
        // 0xFF can never begin a UTF-8 sequence, and must not become U+FFFD.
        let bytes = b"ok\xFFtail";
        match TextFile::open(bytes) {
            Err(TextFileError::NotUtf8 { at }) => assert_eq!(at, 2),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_utf16_bom_is_refused_by_name() {
        match TextFile::open(b"\xFF\xFEh\x00i\x00") {
            Err(TextFileError::Unsupported { encoding }) => assert_eq!(encoding, "UTF-16LE"),
            other => panic!("expected an unsupported encoding, got {other:?}"),
        }
    }

    #[test]
    fn crlf_is_normalised_for_editing_and_reproduced_on_write() {
        let bytes = b"one\r\ntwo\r\n";
        let file = TextFile::open(bytes).expect("CRLF UTF-8");
        assert_eq!(file.text(), "one\ntwo\n");
        assert_eq!(file.endings().crlf(), 2);
        assert!(!file.endings().is_mixed());
        assert_eq!(file.endings().ending(), Ending::CrLf);
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn mixed_line_endings_are_reported_and_cannot_be_written_by_default() {
        // "one LF, one CRLF, one CR", the three kinds in one file.
        let file = TextFile::from_text("one\ntwo\r\nthree\rfour");
        assert!(file.endings().is_mixed());
        assert_eq!(file.endings().lf(), 1);
        assert_eq!(file.endings().crlf(), 1);
        assert_eq!(file.endings().cr(), 1);
        // Reading normalises every ending, so the text is uniform and no byte is lost.
        assert_eq!(file.text(), "one\ntwo\nthree\nfour");
        // Writing is refused, because one ending would change the file.
        match file.encode() {
            Err(TextFileError::MixedLineEndings { lf, crlf, cr }) => {
                assert_eq!((lf, crlf, cr), (1, 1, 1));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        // Choosing an ending is explicit, and then it works.
        assert_eq!(file.encode_with(Ending::Lf), b"one\ntwo\nthree\nfour");
        assert_eq!(
            file.encode_with(Ending::CrLf),
            b"one\r\ntwo\r\nthree\r\nfour"
        );
    }

    #[test]
    fn indentation_is_read_as_tabs_or_as_a_space_width() {
        assert_eq!(Indentation::detect("\tone\n\ttwo\n"), Indentation::Tabs);
        assert_eq!(
            Indentation::detect("one\n    two\n        three\n"),
            Indentation::Spaces { width: 4 }
        );
        // The unit is the greatest common divisor, so 2 and 6 spaces read as 2.
        assert_eq!(
            Indentation::detect("one\n  two\n      three\n"),
            Indentation::Spaces { width: 2 }
        );
        // A file with no indentation at all falls back to four spaces, stated rather than guessed.
        assert_eq!(
            Indentation::detect("one\ntwo\n"),
            Indentation::Spaces { width: 4 }
        );
    }

    #[test]
    fn the_dominant_ending_is_the_one_a_new_line_gets() {
        let endings = LineEndings::detect("a\nb\nc\r\n");
        assert_eq!(endings.lf(), 2);
        assert_eq!(endings.crlf(), 1);
        assert_eq!(endings.ending(), Ending::Lf);
        assert!(endings.is_mixed());
        assert_eq!(Ending::from_name("crlf"), Some(Ending::CrLf));
        assert_eq!(Ending::from_name("nope"), None);
    }
}
