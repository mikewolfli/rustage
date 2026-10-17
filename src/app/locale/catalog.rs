// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A language pack and the catalog that reads it (the designer half of roadmap task T24.03).
//!
//! A pack is a table of `key -> text` for one language, and a catalog is a pack the designer looks
//! text up in. Three rules keep the two honest:
//!
//! * **A missing key is a distinct answer.** [`Text::Missing`] carries the key, and [`Text::render`]
//!   hands the key back, so a string nobody translated shows up as a key rather than as an empty line
//!   — the same "a failure must not look like a success" rule the rest of the designer follows
//!   (principle #22).
//! * **A language the build does not bundle is refused, not approximated.** [`Catalog::for_language`]
//!   answers [`NotBundled`] for one, and [`select`] refuses an *explicit* request for it while letting
//!   an *unknown* system language fall back to [`Language::DEFAULT`] with the request recorded — the
//!   two are different facts and are reported differently.
//! * **A pack round-trips through its written form.** [`Pack::encode`] and [`Pack::decode`] are the one
//!   pair that reads and writes the pack file, so the English pack `--locale-report` produces can be
//!   read back and checked against what was produced.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use crate::host::locale::{DEFAULT, Language};

/// The text for a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Text<'a> {
    /// The key has text.
    Found(&'a str),
    /// The key has none; the key itself is what a caller can show.
    Missing {
        /// The key nothing was found for.
        key: &'a str,
    },
}

impl<'a> Text<'a> {
    /// Whether the key had text.
    pub fn is_found(&self) -> bool {
        matches!(self, Self::Found(_))
    }

    /// The text, or the key when there is none.
    pub fn render(&self) -> &'a str {
        match self {
            Self::Found(text) => text,
            Self::Missing { key } => key,
        }
    }
}

/// One language's messages.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pack {
    language: Language,
    messages: BTreeMap<String, String>,
}

impl Pack {
    /// An empty pack for a language.
    pub fn new(language: Language) -> Self {
        Self {
            language,
            messages: BTreeMap::new(),
        }
    }

    /// The language the pack is for.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Adds a message, answering the text it replaced, if any.
    pub fn insert(&mut self, key: impl Into<String>, text: impl Into<String>) -> Option<String> {
        self.messages.insert(key.into(), text.into())
    }

    /// The text for a key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.messages.get(key).map(String::as_str)
    }

    /// How many messages the pack holds.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the pack is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Every entry, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.messages
            .iter()
            .map(|(key, text)| (key.as_str(), text.as_str()))
    }

    /// The pack written as its file form.
    pub fn encode(&self) -> String {
        let mut text = String::from("rustage-locale 1\n");
        text.push_str(&format!("language {}\n", self.language.tag()));
        for (key, message) in &self.messages {
            text.push_str(key);
            text.push('\t');
            text.push_str(&escape(message));
            text.push('\n');
        }
        text
    }

    /// The pack read back from its file form.
    pub fn decode(text: &str) -> Result<Self, Fault> {
        let mut lines = text.lines();
        let header = lines.next().unwrap_or_default();
        if header.trim() != "rustage-locale 1" {
            return Err(Fault::BadHeader {
                line: header.to_owned(),
            });
        }
        let language_line = lines.next().unwrap_or_default();
        let Some(tag) = language_line.trim().strip_prefix("language ") else {
            return Err(Fault::BadHeader {
                line: language_line.to_owned(),
            });
        };
        let Some(language) = Language::parse(tag) else {
            return Err(Fault::UnknownLanguage {
                tag: tag.trim().to_owned(),
            });
        };
        let mut pack = Self::new(language);
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let Some((key, message)) = line.split_once('\t') else {
                return Err(Fault::BadEntry {
                    line: line.to_owned(),
                });
            };
            pack.insert(key.to_owned(), unescape(message));
        }
        Ok(pack)
    }
}

/// Why a pack could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The first line is not the pack header.
    BadHeader {
        /// The line that was there.
        line: String,
    },
    /// The pack names a language this build does not know.
    UnknownLanguage {
        /// The tag it named.
        tag: String,
    },
    /// A line is not `key<TAB>text`.
    BadEntry {
        /// The line that was there.
        line: String,
    },
}

impl Fault {
    /// The fault in one line.
    pub fn message(&self) -> String {
        match self {
            Self::BadHeader { line } => {
                format!("the pack header is {line:?}, not `rustage-locale 1`")
            }
            Self::UnknownLanguage { tag } => {
                format!("the pack names language {tag:?}, which this build does not know")
            }
            Self::BadEntry { line } => {
                format!("the pack line {line:?} is not `key<TAB>text`")
            }
        }
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// Why a language could not be used.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NotBundled {
    /// The language that was asked for.
    pub language: Language,
}

impl NotBundled {
    /// The refusal in one line.
    pub fn message(&self) -> String {
        format!(
            "this build bundles only {}; the {} pack is not included",
            bundled_names(),
            self.language.name()
        )
    }
}

impl fmt::Display for NotBundled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// The languages this build bundles, named for a message.
fn bundled_names() -> String {
    Language::ALL
        .iter()
        .filter(|language| language.bundled())
        .map(|language| language.name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A pack the designer reads text from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Catalog {
    pack: Pack,
}

impl Catalog {
    /// The catalog for a language, or why this build has none.
    pub fn for_language(language: Language) -> Result<Self, NotBundled> {
        if !language.bundled() {
            return Err(NotBundled { language });
        }
        Ok(Self {
            pack: super::vocabulary::pack(language),
        })
    }

    /// The catalog for the bundled language.
    ///
    /// This is the door a caller that has no language to hand goes through; with one bundled language
    /// the answer is English, and the parameter arrives with the second pack rather than being invented
    /// now.
    pub fn bundled() -> Self {
        Self {
            pack: super::vocabulary::pack(DEFAULT),
        }
    }

    /// The catalog for the language the process is using.
    ///
    /// This is the door the designer's own diagnostics go through ([`crate::app::commands::command`]):
    /// it resolves the installed interface language to its pack and falls back to the bundled one when
    /// there is none, so a translated pack changes the diagnostic a person reads without any caller
    /// having to know which pack is present.
    pub fn current() -> Self {
        Self::for_language(installed().language()).unwrap_or_else(|_| Self::bundled())
    }

    /// The catalog built on a pack, for reading a translated pack back in tests and reports.
    pub fn from_pack(pack: Pack) -> Self {
        Self { pack }
    }

    /// The language the catalog holds.
    pub fn language(&self) -> Language {
        self.pack.language()
    }

    /// The pack behind the catalog.
    pub fn pack(&self) -> &Pack {
        &self.pack
    }

    /// The text for a key.
    pub fn text<'a>(&'a self, key: &'a str) -> Text<'a> {
        match self.pack.get(key) {
            Some(text) => Text::Found(text),
            None => Text::Missing { key },
        }
    }

    /// The text for a key with its `{name}` placeholders filled in.
    ///
    /// A key that has no text answers with the key and a note, so an untranslated diagnostic is
    /// visibly untranslated rather than silently empty.
    pub fn fill(&self, key: &str, arguments: &[(&str, &str)]) -> String {
        match self.text(key) {
            Text::Found(template) => substitute(template, arguments),
            Text::Missing { key } => format!("{key} (has no text)"),
        }
    }
}

/// Replaces each `{name}` in a template with its argument.
fn substitute(template: &str, arguments: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in arguments {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// How the interface language was decided.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Selection {
    /// A bundled language was chosen.
    Chosen {
        /// The language.
        language: Language,
    },
    /// The system asked for a language this build does not bundle, so it fell back.
    FellBack {
        /// What the system asked for.
        requested: Language,
        /// What was used instead.
        resolved: Language,
        /// Why.
        reason: String,
    },
    /// An explicit request that cannot be honoured.
    Refused {
        /// The tag that was asked for.
        tag: String,
        /// Why.
        reason: String,
    },
}

impl Selection {
    /// The language the interface will use.
    pub fn language(&self) -> Language {
        match self {
            Self::Chosen { language } => *language,
            Self::FellBack { resolved, .. } => *resolved,
            // A refusal is reported and the process does not continue; the fallback is what a caller
            // would see if it did.
            Self::Refused { .. } => DEFAULT,
        }
    }

    /// Whether the selection is a refusal.
    pub fn is_refused(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }

    /// The selection in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Chosen { language } => {
                format!(
                    "interface language {} ({})",
                    language.name(),
                    language.tag()
                )
            }
            Self::FellBack {
                requested,
                resolved,
                reason,
            } => format!(
                "interface language {} ({}) — asked for {}: {reason}",
                resolved.name(),
                resolved.tag(),
                requested.name()
            ),
            Self::Refused { tag, reason } => format!("language {tag:?} is not available: {reason}"),
        }
    }
}

/// Decides the interface language from an explicit request or from the system.
///
/// An explicit request for a language this build does not bundle is refused; a *system* language it
/// does not bundle falls back to [`DEFAULT`] with the request recorded, because following the system
/// is the default and a machine set to a language the build cannot speak must still start.
pub fn select(requested: Option<&str>) -> Selection {
    match requested {
        Some(tag) => match Language::parse(tag) {
            None => Selection::Refused {
                tag: tag.to_owned(),
                reason: "this build does not know that language".to_owned(),
            },
            Some(language) if language.bundled() => Selection::Chosen { language },
            Some(language) => Selection::Refused {
                tag: tag.to_owned(),
                reason: format!(
                    "this build bundles only {}; the {} pack is not included",
                    bundled_names(),
                    language.name()
                ),
            },
        },
        None => match Language::system() {
            Some(language) if language.bundled() => Selection::Chosen { language },
            Some(language) => Selection::FellBack {
                requested: language,
                resolved: DEFAULT,
                reason: format!("the {} pack is not bundled", language.name()),
            },
            None => Selection::Chosen { language: DEFAULT },
        },
    }
}

/// The interface language the process was started with.
///
/// It is installed once, the way the logger is: the interface language is a choice that holds for
/// the whole process, so a command does not carry it as a parameter and a report can name the same
/// language the rest of the process is using.
static INSTALLED: OnceLock<Selection> = OnceLock::new();

/// Records the interface language for the process. Later calls are ignored, matching the logger.
pub fn install(selection: Selection) {
    let _ = INSTALLED.set(selection);
}

/// The interface language the process is using, or the default when nothing was installed (a unit
/// test that never reaches `main`).
pub fn installed() -> Selection {
    INSTALLED
        .get()
        .cloned()
        .unwrap_or(Selection::Chosen { language: DEFAULT })
}

/// Writes a `\t` and a newline in text into escapes the pack line form can carry.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// Reads back what [`escape`] wrote.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Catalog, Fault, Pack, Selection, Text, select};
    use crate::host::locale::Language;

    fn pack() -> Pack {
        let mut pack = Pack::new(Language::English);
        pack.insert("cmd.file.save.title", "Save");
        pack.insert("diag.example", "there is no command {id}");
        pack
    }

    #[test]
    fn a_missing_key_renders_as_the_key_not_as_nothing() {
        let catalog = Catalog::from_pack(pack());
        let missing = catalog.text("nothing.here");
        assert_eq!(
            missing,
            Text::Missing {
                key: "nothing.here"
            }
        );
        assert!(!missing.is_found());
        assert_eq!(missing.render(), "nothing.here");
        assert!(catalog.fill("nothing.here", &[]).contains("has no text"));
    }

    #[test]
    fn a_found_key_renders_its_text() {
        let catalog = Catalog::from_pack(pack());
        assert_eq!(catalog.text("cmd.file.save.title").render(), "Save");
        assert!(catalog.text("cmd.file.save.title").is_found());
    }

    #[test]
    fn a_template_fills_each_placeholder() {
        let catalog = Catalog::from_pack(pack());
        assert_eq!(
            catalog.fill("diag.example", &[("id", "file.save")]),
            "there is no command file.save"
        );
    }

    #[test]
    fn a_pack_round_trips_through_its_file_form_escapes_and_all() {
        let mut pack = Pack::new(Language::SimplifiedChinese);
        pack.insert("k.one", "plain");
        pack.insert("k.two", "a\ttab and a\nnewline and a \\ backslash");
        let text = pack.encode();
        assert_eq!(Pack::decode(&text).expect("a pack"), pack);
    }

    #[test]
    fn a_pack_with_a_bad_header_or_line_is_refused() {
        assert!(matches!(
            Pack::decode("nonsense\nlanguage en\n"),
            Err(Fault::BadHeader { .. })
        ));
        assert!(matches!(
            Pack::decode("rustage-locale 1\nlanguage en\nno-tab-here\n"),
            Err(Fault::BadEntry { .. })
        ));
        assert!(matches!(
            Pack::decode("rustage-locale 1\nlanguage klingon\n"),
            Err(Fault::UnknownLanguage { .. })
        ));
        assert!(
            Fault::BadHeader {
                line: "x".to_owned()
            }
            .message()
            .contains("header")
        );
    }

    #[test]
    fn the_bundled_catalog_speaks_english_and_is_not_empty() {
        let catalog = Catalog::bundled();
        assert_eq!(catalog.language(), Language::English);
        assert!(!catalog.pack().is_empty());
    }

    #[test]
    fn an_unbundled_language_is_refused_not_approximated() {
        let refusal = Catalog::for_language(Language::SimplifiedChinese).expect_err("a refusal");
        assert_eq!(refusal.language, Language::SimplifiedChinese);
        assert!(refusal.message().contains("not included"));
        assert!(Catalog::for_language(Language::English).is_ok());
    }

    #[test]
    fn an_explicit_request_for_an_unbundled_language_is_refused() {
        let selection = select(Some("zh-Hans"));
        assert!(selection.is_refused());
        assert!(selection.render().contains("not available"));
    }

    #[test]
    fn an_explicit_request_that_is_unknown_is_refused_too() {
        assert!(select(Some("klingon")).is_refused());
        assert!(select(Some("de")).is_refused());
    }

    #[test]
    fn an_explicit_request_for_a_bundled_language_is_chosen() {
        assert_eq!(
            select(Some("en-US")),
            Selection::Chosen {
                language: Language::English
            }
        );
    }

    #[test]
    fn a_system_language_that_is_not_bundled_falls_back_with_the_request_recorded() {
        // Whatever the machine is set to, selecting with no explicit request must never be a refusal.
        let selection = select(None);
        assert!(!selection.is_refused());
        assert_eq!(selection.language(), Language::English);
    }
}
