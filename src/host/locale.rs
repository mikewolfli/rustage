// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The interface language the designer runs in, and the direction it reads (the platform half of
//! roadmap task T24.03; project principle #67).
//!
//! The principle is that the designer's interface is multilingual and *follows the system by
//! default*, that only the languages the build actually bundles are offered, and that a language it
//! does not bundle is never presented as supported. This module owns the part of that which is a
//! fact about the machine rather than about the designer:
//!
//! * **A [`Language`] is a known interface language, not a free-form tag.** [`Language::ALL`] is the
//!   set the build knows about; a tag the system names that is not in it is [`Language::from_locale`]
//!   answering `None`, so "unknown" is distinguishable from "known but not bundled".
//! * **Which languages are bundled is explicit.** [`Language::bundled`] answers whether this build
//!   carries a pack for one; English and Simplified Chinese do (T24.03). A request for an unbundled
//!   language is refused where it is made, never silently answered in English.
//! * **The system's language is read, not assumed.** [`Language::system`] reads the conventional
//!   locale variables and reports what it recognised, so "follow the system" has a value behind it.
//! * **Direction is a property of the language, not of the text.** [`Direction`] is here so a
//!   right-to-left pack lands as a language fact rather than as per-string mirroring.

#![allow(dead_code)]

/// The direction a language reads in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Left to right.
    LeftToRight,
    /// Right to left.
    RightToLeft,
}

impl Direction {
    /// The direction's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::LeftToRight => "ltr",
            Self::RightToLeft => "rtl",
        }
    }
}

/// An interface language this build knows about.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Language {
    /// English.
    English,
    /// Simplified Chinese.
    SimplifiedChinese,
}

/// The language used when nothing else can be.
pub const DEFAULT: Language = Language::English;

impl Language {
    /// Every language this build knows about.
    pub const ALL: [Self; 2] = [Self::English, Self::SimplifiedChinese];

    /// The language's tag, as written in a pack and on the command line.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-Hans",
        }
    }

    /// The language's own name, in English, for a report.
    pub const fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::SimplifiedChinese => "Simplified Chinese",
        }
    }

    /// The direction the language reads in.
    pub const fn direction(self) -> Direction {
        match self {
            Self::English | Self::SimplifiedChinese => Direction::LeftToRight,
        }
    }

    /// Whether this build carries a language pack for it.
    ///
    /// Both known languages are bundled: English is derived from the identified vocabulary and
    /// Simplified Chinese is translated against the same keys (T24.03). A language added to
    /// [`Self::ALL`] before its pack exists must answer `false` here, so it is refused where it is
    /// requested instead of being offered as if it were translated.
    pub const fn bundled(self) -> bool {
        matches!(self, Self::English | Self::SimplifiedChinese)
    }

    /// The language a tag names, when this build knows it.
    ///
    /// Accepts a plain tag (`en`, `zh`), a tag with a region or script (`en-US`, `zh-Hans`,
    /// `zh_CN`), and the `C`/`POSIX` locale. Simplified Chinese is recognised unless the tag names a
    /// traditional script, because offering simplified text for `zh-Hant` would be the wrong
    /// language rather than a missing one.
    pub fn parse(text: &str) -> Option<Self> {
        let normalized = text.trim().to_ascii_lowercase().replace('_', "-");
        let stem = normalized.split('.').next().unwrap_or(&normalized);
        let primary = stem.split(['-', '@']).next().unwrap_or(stem);
        match primary {
            "en" => Some(Self::English),
            "zh" => {
                let traditional = ["hant", "tw", "hk", "mo"];
                if traditional.iter().any(|part| stem.contains(part)) {
                    None
                } else {
                    Some(Self::SimplifiedChinese)
                }
            }
            _ => None,
        }
    }

    /// The language a POSIX locale value names, when it names one this build knows.
    pub fn from_locale(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() || value == "C" || value == "POSIX" {
            return None;
        }
        Self::parse(value)
    }

    /// The language the system is set to, when this build recognises it.
    ///
    /// Reads the conventional locale variables in their usual precedence and answers `None` when none
    /// of them names a language this build knows — which is "the system asked for something else",
    /// not "the system speaks English".
    pub fn system() -> Option<Self> {
        for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            let Ok(value) = std::env::var(variable) else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            if value == "C" || value == "POSIX" {
                return None;
            }
            return Self::from_locale(value);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT, Direction, Language};

    #[test]
    fn a_plain_tag_names_its_language() {
        assert_eq!(Language::parse("en"), Some(Language::English));
        assert_eq!(Language::parse("zh"), Some(Language::SimplifiedChinese));
    }

    #[test]
    fn a_region_or_script_tag_names_its_language() {
        for tag in ["en-US", "en_GB.UTF-8", "en-AU"] {
            assert_eq!(Language::parse(tag), Some(Language::English), "{tag}");
        }
        for tag in ["zh-Hans", "zh_CN", "zh-SG", "zh-Hans-CN"] {
            assert_eq!(
                Language::parse(tag),
                Some(Language::SimplifiedChinese),
                "{tag}"
            );
        }
    }

    #[test]
    fn a_traditional_script_is_not_simplified_chinese() {
        for tag in ["zh-Hant", "zh_TW", "zh-HK", "zh-MO"] {
            assert_eq!(Language::parse(tag), None, "{tag} is not simplified");
        }
    }

    #[test]
    fn an_unknown_or_c_locale_names_no_language() {
        assert_eq!(Language::parse("de"), None);
        assert_eq!(Language::parse("klingon"), None);
        assert_eq!(Language::parse(""), None);
        assert_eq!(Language::from_locale("C"), None);
        assert_eq!(Language::from_locale("POSIX"), None);
        assert_eq!(Language::from_locale(""), None);
        assert_eq!(Language::from_locale("de_DE.UTF-8"), None);
        assert_eq!(
            Language::from_locale("zh_CN.UTF-8"),
            Some(Language::SimplifiedChinese)
        );
    }

    #[test]
    fn both_known_languages_are_bundled() {
        assert!(Language::English.bundled());
        assert!(Language::SimplifiedChinese.bundled());
        assert!(DEFAULT.bundled(), "the fallback must always be available");
    }

    #[test]
    fn tags_and_names_are_stable_and_every_language_reads_left_to_right() {
        assert_eq!(Language::English.tag(), "en");
        assert_eq!(Language::SimplifiedChinese.tag(), "zh-Hans");
        assert_eq!(Language::SimplifiedChinese.name(), "Simplified Chinese");
        for language in Language::ALL {
            assert_eq!(language.direction(), Direction::LeftToRight);
        }
        assert_eq!(Direction::RightToLeft.name(), "rtl");
    }

    #[test]
    fn a_tag_round_trips_through_its_language() {
        for language in Language::ALL {
            assert_eq!(Language::parse(language.tag()), Some(language));
        }
    }
}
