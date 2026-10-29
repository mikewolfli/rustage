// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generated application's language resources, key audit, text preview and direction (the host
//! half of roadmap task T24.04).
//!
//! This module owns the *target* application's catalogue. It is deliberately separate from
//! [`crate::app::locale`], which holds the designer's own interface vocabulary: translating the
//! designer does not translate the application it generates, and neither side may report the
//! other's progress as its own.
//!
//! # What is built on the pinned library
//!
//! [`Resources`] wraps [`rust_widgets::i18n::I18nManager`] and uses its real lookup semantics:
//! plural forms are keyed by the number the caller passes (not by a CLDR category name), a
//! contextual lookup accepts an entry whose own `context` is `None`, and a missing active-language
//! entry falls back to `en` before echoing the key. The module does not reimplement any of that.
//!
//! Translation JSON is parsed with the host's own [`crate::project::record::json`] reader rather
//! than a second serde shape, so a malformed file is refused with a reason that names the key and
//! the member at fault. Parsed entries are injected through
//! [`I18nManager::inject_translations`](rust_widgets::i18n::I18nManager::inject_translations),
//! which is the embedded-data path and therefore registers **no file path** with the manager.
//!
//! # Known limits, stated rather than papered over
//!
//! * **No per-language file access upstream.** `I18nManager` exposes only the union of keys
//!   ([`audit_keys`](rust_widgets::i18n::I18nManager::audit_keys)) and the number of loaded files
//!   ([`translation_count`](rust_widgets::i18n::I18nManager::translation_count)); it has no getter
//!   for one language's catalogue. [`Resources`] therefore keeps a host-side
//!   `BTreeMap<String, BTreeSet<String>>` of the keys each `load_json` supplied. The audit is only
//!   as complete as that index, which is complete for every catalogue this module loaded and empty
//!   for one it did not.
//! * **Live reload is real for a catalogue loaded from a file, and absent for injected text.**
//!   [`Resources::load_file`] goes through the manager's path-based loader, so `reload` re-reads the
//!   file when it changes. [`Resources::load_json`] calls `inject_translations`, which registers no
//!   path; a later [`Resources::reload`] receives the manager's own "no translation file is
//!   registered" error, which is propagated. Re-reading from disk is not faked.
//! * **Direction is a fact, not rendering.** [`direction`] answers which end of a line is the
//!   beginning for a locale tag. Upstream complex-script shaping is not implemented, and this
//!   function does not mirror padding, icons, scrollbars or any other layout by itself. A caller
//!   that only asks for the direction gets one honest bit, not an RTL implementation.
//! * **Preview is a character count, not a measurement.** [`Resources::preview`] wraps by Unicode
//!   scalar values. It knows nothing about font metrics, kerning, grapheme clusters or shaping, so
//!   its line count must never be presented as a measured width.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rust_widgets::core::TextDirection;
use rust_widgets::i18n::{I18nManager, Translation, TranslationFile};

use crate::project::record::json::{Json, parse};

/// The source language the library falls back to when the active language has no entry.
pub const ENGLISH: &str = "en";

/// The most characters a [`Preview`] retains.
///
/// A preview is for showing long text, not for reproducing a whole document; the cap keeps one
/// pathological message from turning a preview into a large allocation. It is a preview budget,
/// not a translation limit: the catalogue still holds the full message.
pub const MAX_PREVIEW_CHARS: usize = 4096;

/// The result of loading one translation catalogue.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loaded {
    /// The language tag the file declared, stored as written (the library does not normalise it).
    pub language: String,
    /// How many keys the file supplied, including plural and contextual entries.
    pub keys: usize,
    /// Whether a catalogue for this exact tag was already indexed, so this load replaced it.
    ///
    /// This is read from the host-side key index, which is populated by [`Resources::load_json`].
    pub replaced: bool,
}

/// The outcome of asking for a language.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Selection {
    /// The requested catalogue is loaded and is now active.
    Loaded {
        /// The tag that is now active.
        language: String,
    },
    /// The requested catalogue is absent, but `en` is loaded, so lookups fall back to `en`.
    ///
    /// The requested tag is made active so the manager's own fallback path is exercised; the
    /// result never claims that a translation for the requested language exists.
    FallsBackToEnglish {
        /// The tag that was requested and is now active with no catalogue of its own.
        requested: String,
        /// The tag that will answer lookups.
        fallback: String,
    },
    /// Neither the requested catalogue nor `en` is loaded, so the selection was refused.
    ///
    /// Refusing (instead of switching and returning raw keys) follows the same rule as
    /// [`crate::app::locale::catalog`]: an explicit request for a language the build does not have
    /// is an answer of its own, not an approximation. The previous selection stays active.
    Unavailable {
        /// The tag that could not be selected.
        requested: String,
    },
}

/// What a key audit found between `en` and one language.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Audit {
    /// The language the audit describes, exactly as passed to [`Resources::audit`].
    pub language: String,
    /// Keys present in `en` and absent from `language`, sorted.
    pub missing: Vec<String>,
    /// Keys present in `language` and absent from `en`, sorted.
    pub extra: Vec<String>,
}

impl Audit {
    /// How many keys `language` lacks.
    pub fn missing_count(&self) -> usize {
        self.missing.len()
    }

    /// How many keys `language` has that `en` does not.
    pub fn extra_count(&self) -> usize {
        self.extra.len()
    }

    /// Whether `language` covers `en` exactly, with nothing missing and nothing extra.
    pub fn is_exact(&self) -> bool {
        self.missing.is_empty() && self.extra.is_empty()
    }
}

/// A character-count preview of one translated message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Preview {
    /// The preview text: the message itself, or its first [`MAX_PREVIEW_CHARS`] characters.
    pub text: String,
    /// The character (Unicode scalar value) count of the *message*, before the preview cap.
    pub characters: usize,
    /// `text` divided into lines of at most the requested characters; one line for `0`.
    pub lines: Vec<String>,
    /// Whether `text` is a prefix of the message because the message exceeded the preview cap.
    pub truncated: bool,
}

/// The generated application's loaded catalogues and their lookups.
///
/// A new value starts with no catalogues at all — `en` is not supplied implicitly. A caller that
/// wants fallback must load an `en` catalogue first, which keeps "the build ships English" a fact
/// the build can check rather than an assumption inside this type.
pub struct Resources {
    manager: I18nManager,
    /// Per-language key index kept because the pinned manager has no per-language accessor.
    keys: BTreeMap<String, BTreeSet<String>>,
}

impl Resources {
    /// A resource set with no catalogues loaded.
    pub fn new() -> Self {
        Self {
            manager: I18nManager::new(),
            keys: BTreeMap::new(),
        }
    }

    /// Reads one library `TranslationFile` shape and injects it.
    ///
    /// The accepted shape is `{ "language": "de", "translations": { "<key>": { "message": "...",
    /// "context": "...", "plural": { "1": "...", "2": "..." } } } }`. `context` and `plural` are
    /// optional; an unknown member is ignored, matching the library's serde definition, which
    /// accepts extra members rather than rejecting the file. A shape this module cannot interpret
    /// is refused with the key and member named, and nothing is injected.
    pub fn load_json(&mut self, text: &str) -> Result<Loaded, String> {
        let (file, keys) = parse_catalogue(text)?;
        Ok(self.install(file, keys))
    }

    /// Loads one catalogue from a file and registers the path for later reloads.
    ///
    /// Unlike [`Self::load_json`], this goes through the manager's path-based loader, so
    /// [`Self::reload`] can genuinely re-read the file when it changes (hot reload). The text is
    /// validated here first, so a file this module cannot index is refused before the manager sees
    /// it; the manager then parses the same file, and its answer is not treated as a success unless
    /// it returned one.
    ///
    /// The manager identifies a language by the **file name**, so the file must be named
    /// `<language>.json` (`de.json` for a file declaring `de`). A mismatch is refused by the
    /// manager with its own reason; this method does not rename or copy the file to hide it.
    pub fn load_file(&mut self, path: &Path) -> Result<Loaded, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let (file, keys) = parse_catalogue(&text)?;
        let name = path.to_string_lossy().into_owned();
        self.manager
            .load_translations(&name)
            .map_err(|error| format!("cannot load {name}: {error}"))?;
        Ok(self.install(file, keys))
    }

    /// Records a parsed catalogue and its key index.
    fn install(&mut self, file: TranslationFile, keys: BTreeSet<String>) -> Loaded {
        let language = file.language.clone();
        let key_count = keys.len();
        let replaced = self.keys.contains_key(&language);
        // The manager takes the tag separately from the file it stores. Both come from the same
        // parsed `language` member, so the manager key and the file's declared language cannot
        // disagree here.
        self.manager.inject_translations(language.clone(), file);
        self.keys.insert(language.clone(), keys);
        Loaded {
            language,
            keys: key_count,
            replaced,
        }
    }

    /// Requests a language and reports exactly what lookups will now do.
    ///
    /// Tags are compared verbatim, because [`TranslationFile::language`] is stored as written and
    /// documents that `"en"` and `"en-US"` are different languages. Use the same spelling that was
    /// loaded; [`direction`] is the one place that normalises case and separators.
    pub fn select(&mut self, language: &str) -> Selection {
        if self.keys.contains_key(language) {
            self.manager.set_language(language);
            Selection::Loaded {
                language: language.to_string(),
            }
        } else if self.keys.contains_key(ENGLISH) {
            self.manager.set_language(language);
            Selection::FallsBackToEnglish {
                requested: language.to_string(),
                fallback: ENGLISH.to_string(),
            }
        } else {
            Selection::Unavailable {
                requested: language.to_string(),
            }
        }
    }

    /// The translation for `key`, using the manager's context and plural lookup.
    ///
    /// The result is the active-language entry, else the `en` entry, else the key itself. That
    /// last case is the library's documented miss signal: a caller can detect it by comparing the
    /// result with `key`, and no empty string is ever substituted for a missing translation.
    pub fn translate(&self, key: &str, context: Option<&str>, count: u32) -> String {
        self.manager.translate_with_context(key, context, count)
    }

    /// Compares one loaded language against `en`.
    ///
    /// A key present in `en` but absent here is `missing`; a key present here but absent from `en`
    /// is `extra`. Both lists come from the host-side key index and are sorted by `String` order.
    ///
    /// An unloaded `language` has no keys, so every `en` key is missing; an unloaded `en` has no
    /// keys, so every key in `language` is extra. Neither case is reported as a clean audit.
    pub fn audit(&self, language: &str) -> Audit {
        let empty = BTreeSet::new();
        let language_keys = self.keys.get(language).unwrap_or(&empty);
        let english_keys = self.keys.get(ENGLISH).unwrap_or(&empty);

        Audit {
            language: language.to_string(),
            missing: english_keys.difference(language_keys).cloned().collect(),
            extra: language_keys.difference(english_keys).cloned().collect(),
        }
    }

    /// Re-reads one language through the manager's reentrant-safe reload path.
    ///
    /// [`I18nManager::reload_translation_quiet`](rust_widgets::i18n::I18nManager::reload_translation_quiet)
    /// is used rather than `reload_translation`, so an explicit reload made from inside a
    /// hot-reload drain cannot announce itself back into the channel being drained.
    ///
    /// # Which catalogues can be reloaded
    ///
    /// A catalogue loaded with [`Self::load_file`] registered its path, so this genuinely re-reads
    /// the file and the next lookup sees the new text. A catalogue injected with
    /// [`Self::load_json`] registered no path, and the manager's own "no translation file is
    /// registered" reason is propagated unchanged (with the language named): an embedded catalogue
    /// has no file to re-read, and that refusal is the real state rather than a claimed success.
    pub fn reload(&mut self, language: &str) -> Result<(), String> {
        self.manager
            .reload_translation_quiet(language)
            .map_err(|reason| format!("language {language:?} could not be reloaded: {reason}"))
    }

    /// A character-count preview of one translation.
    ///
    /// `max_chars_per_line == 0` means "no wrapping": the whole preview is one line. A positive
    /// value splits by Unicode scalar values, never by bytes, so a CJK character counts as one and
    /// each line holds at most that many scalars. An empty message still produces one empty line.
    ///
    /// This is an estimate. It does not measure glyphs, account for grapheme clusters (a combining
    /// sequence or an emoji ZWJ sequence counts per scalar), shape complex scripts, or apply font
    /// metrics, so a caller must not present `lines` as measured text width.
    pub fn preview(
        &self,
        key: &str,
        context: Option<&str>,
        count: u32,
        max_chars_per_line: usize,
    ) -> Preview {
        let message = self.translate(key, context, count);
        let characters = message.chars().count();
        let truncated = characters > MAX_PREVIEW_CHARS;
        let text = if truncated {
            message.chars().take(MAX_PREVIEW_CHARS).collect()
        } else {
            message.clone()
        };

        let lines = if max_chars_per_line == 0 || text.is_empty() {
            vec![text.clone()]
        } else {
            let preview_characters: Vec<char> = text.chars().collect();
            preview_characters
                .chunks(max_chars_per_line)
                .map(|line| line.iter().collect())
                .collect()
        };

        Preview {
            text,
            characters,
            lines,
            truncated,
        }
    }
}

impl Default for Resources {
    fn default() -> Self {
        Self::new()
    }
}

/// The writing direction for a locale tag.
///
/// The primary subtag (the part before `-` or `_`) is matched case-insensitively against the
/// right-to-left languages this build knows: Arabic, Hebrew, Persian, Urdu, Yiddish, Divehi,
/// Pashto and Sindhi. Region and script suffixes are ignored, so `ar-EG`, `he_IL`, `fa-IR` and
/// `ur-PK` are right to left, while everything else — including an empty tag — is left to right.
///
/// # What this does not do
///
/// This is a direction fact only. Upstream does not implement complex-script shaping, and this
/// function does not mirror layout, reorder icons, move scrollbars or change padding. A caller
/// must apply the returned direction itself, and a bidirectional text run still needs the shaping
/// support that is missing upstream; treating this answer as "RTL is supported" would overstate
/// what the pinned library can do.
pub fn direction(tag: &str) -> TextDirection {
    let primary = tag
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    if matches!(
        primary.as_str(),
        "ar" | "he" | "fa" | "ur" | "yi" | "dv" | "ps" | "sd"
    ) {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    }
}

/// Parses and validates one library `TranslationFile` shape, returning it with its key index.
fn parse_catalogue(text: &str) -> Result<(TranslationFile, BTreeSet<String>), String> {
    let value =
        parse(text).map_err(|error| format!("translation file is not valid JSON: {error}"))?;
    let root = value
        .as_object()
        .ok_or_else(|| "translation file root must be a JSON object".to_string())?;

    let language_value = root
        .get("language")
        .ok_or_else(|| "translation file is missing the \"language\" member".to_string())?;
    let language = language_value
        .as_str()
        .ok_or_else(|| {
            "translation file has a \"language\" member that is not a string".to_string()
        })?
        .to_string();

    let translations_value = root.get("translations").ok_or_else(|| {
        format!("translation file for language {language:?} is missing the \"translations\" member")
    })?;
    let translations_object = translations_value.as_object().ok_or_else(|| {
        format!(
            "translation file for language {language:?} has a \"translations\" member that is not an object"
        )
    })?;

    let mut translations = HashMap::new();
    for (key, entry) in translations_object.iter() {
        translations.insert(
            key.to_string(),
            translation_from_json(&language, key, entry)?,
        );
    }

    let keys: BTreeSet<String> = translations.keys().cloned().collect();
    Ok((
        TranslationFile {
            language,
            translations,
        },
        keys,
    ))
}

/// Interprets one `translations` member as a library [`Translation`].
fn translation_from_json(language: &str, key: &str, value: &Json) -> Result<Translation, String> {
    let entry = value.as_object().ok_or_else(|| {
        format!("translation {key:?} in language {language:?} must be a JSON object")
    })?;

    let message = match entry.get("message") {
        Some(Json::String(message)) => message.clone(),
        Some(_) => {
            return Err(format!(
                "translation {key:?} in language {language:?} has a \"message\" member that is not a string"
            ));
        }
        None => {
            return Err(format!(
                "translation {key:?} in language {language:?} is missing the \"message\" member"
            ));
        }
    };

    let context = match entry.get("context") {
        None => None,
        Some(Json::String(context)) => Some(context.clone()),
        Some(_) => {
            return Err(format!(
                "translation {key:?} in language {language:?} has a \"context\" member that is not a string"
            ));
        }
    };

    let plural = match entry.get("plural") {
        None => None,
        Some(value) => {
            let forms = value.as_object().ok_or_else(|| {
                format!(
                    "translation {key:?} in language {language:?} has a \"plural\" member that is not an object"
                )
            })?;
            let mut plural: HashMap<u32, String> = HashMap::new();
            for (raw_category, form) in forms.iter() {
                let category: u32 = raw_category.parse().map_err(|_| {
                    format!(
                        "plural category {raw_category:?} for translation {key:?} in language {language:?} is not a number"
                    )
                })?;
                let form = form.as_str().ok_or_else(|| {
                    format!(
                        "plural form {category} for translation {key:?} in language {language:?} is not a string"
                    )
                })?;
                if plural.insert(category, form.to_string()).is_some() {
                    return Err(format!(
                        "plural category {raw_category:?} for translation {key:?} in language {language:?} repeats category {category}"
                    ));
                }
            }
            Some(plural)
        }
    };

    Ok(Translation {
        context,
        message,
        plural,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plural_lookup_uses_the_number_as_the_category() {
        let mut resources = Resources::new();
        resources
            .load_json(
                r#"{
                    "language": "de",
                    "translations": {
                        "dateien": {
                            "message": "Dateien (keine Pluralform für diese Anzahl)",
                            "plural": {
                                "1": "eine Datei",
                                "2": "zwei Dateien"
                            }
                        }
                    }
                }"#,
            )
            .expect("the catalogue is well formed");
        assert_eq!(
            resources.select("de"),
            Selection::Loaded {
                language: "de".to_string()
            }
        );

        // The pinned library looks a plural form up by the count itself
        // (`plural.get(&count)`), not by selecting a CLDR category first. Category 1 answers
        // count 1, category 2 answers count 2, and count 3 has no form, so the singular
        // `message` is used. Count 1 and count 3 therefore genuinely resolve differently.
        assert_eq!(resources.translate("dateien", None, 1), "eine Datei");
        assert_eq!(resources.translate("dateien", None, 2), "zwei Dateien");
        assert_eq!(
            resources.translate("dateien", None, 3),
            "Dateien (keine Pluralform für diese Anzahl)"
        );
    }

    #[test]
    fn context_filters_an_entry_and_a_context_free_entry_matches_any_context() {
        let mut resources = Resources::new();
        resources
            .load_json(
                r#"{
                    "language": "en",
                    "translations": {
                        "greeting": {
                            "message": "Hello",
                            "context": "formal"
                        },
                        "plain": {
                            "message": "No context"
                        }
                    }
                }"#,
            )
            .expect("the English catalogue is well formed");
        // A second catalogue can give the same key a different context. The library's translation
        // map holds one entry per key, so two simultaneous contexts for one key are not
        // representable in a single file; across catalogues they are, and the lookup follows the
        // active language with the documented `en` fallback.
        resources
            .load_json(
                r#"{
                    "language": "de",
                    "translations": {
                        "greeting": {
                            "message": "Hallo",
                            "context": "casual"
                        }
                    }
                }"#,
            )
            .expect("the German catalogue is well formed");

        resources.select("de");
        assert_eq!(resources.translate("greeting", Some("casual"), 1), "Hallo");
        // `de` has the key only for `casual`, so the `formal` lookup falls through to `en`,
        // where the same key carries the `formal` entry.
        assert_eq!(resources.translate("greeting", Some("formal"), 1), "Hello");
        // A context on the entry that does not match is not served as if it did.
        resources.select("en");
        assert_eq!(
            resources.translate("greeting", Some("casual"), 1),
            "greeting"
        );
        // An entry without a context is "unambiguous on its own" per `Translation::context`, so a
        // contextual lookup still resolves it.
        assert_eq!(
            resources.translate("plain", Some("anything"), 1),
            "No context"
        );
    }

    #[test]
    fn selecting_an_unloaded_language_falls_back_to_english_and_reports_it() {
        let mut resources = Resources::new();
        resources
            .load_json(
                r#"{
                    "language": "en",
                    "translations": {
                        "hello": {
                            "message": "Hello"
                        }
                    }
                }"#,
            )
            .expect("the English catalogue is well formed");

        assert_eq!(
            resources.select("de"),
            Selection::FallsBackToEnglish {
                requested: "de".to_string(),
                fallback: "en".to_string()
            }
        );
        assert_eq!(resources.translate("hello", None, 1), "Hello");
    }

    #[test]
    fn selecting_with_neither_catalogue_is_refused() {
        let mut resources = Resources::new();
        assert_eq!(
            resources.select("de"),
            Selection::Unavailable {
                requested: "de".to_string()
            }
        );
        // Nothing was switched, so the library's default language is still active and a miss is
        // visible as the key rather than as a fabricated translation.
        assert_eq!(resources.translate("hello", None, 1), "hello");
    }

    #[test]
    fn audit_reports_keys_missing_and_extra_against_english() {
        let mut resources = Resources::new();
        resources
            .load_json(
                r#"{
                    "language": "en",
                    "translations": {
                        "shared": {
                            "message": "Shared"
                        },
                        "only.en": {
                            "message": "English only"
                        }
                    }
                }"#,
            )
            .expect("the English catalogue is well formed");
        resources
            .load_json(
                r#"{
                    "language": "de",
                    "translations": {
                        "shared": {
                            "message": "Geteilt"
                        },
                        "only.de": {
                            "message": "Nur Deutsch"
                        }
                    }
                }"#,
            )
            .expect("the German catalogue is well formed");

        let audit = resources.audit("de");
        assert_eq!(audit.language, "de");
        assert_eq!(audit.missing, vec!["only.en".to_string()]);
        assert_eq!(audit.extra, vec!["only.de".to_string()]);
        assert_eq!(audit.missing_count(), 1);
        assert_eq!(audit.extra_count(), 1);
        assert!(!audit.is_exact());

        // A language with no catalogue is not reported as complete: every English key is missing.
        let unloaded = resources.audit("fr");
        assert_eq!(
            unloaded.missing,
            vec!["only.en".to_string(), "shared".to_string()]
        );
        assert!(unloaded.extra.is_empty());
    }

    #[test]
    fn direction_is_right_to_left_for_the_known_primary_subtags() {
        for tag in ["ar", "ar-EG", "he_IL", "fa-IR", "ur-PK"] {
            assert_eq!(
                direction(tag),
                TextDirection::RightToLeft,
                "{tag} must be right to left"
            );
        }
        for tag in ["en", "zh-Hans", "ja"] {
            assert_eq!(
                direction(tag),
                TextDirection::LeftToRight,
                "{tag} must be left to right"
            );
        }
        // Case and separator normalisation are part of the contract.
        assert_eq!(direction("AR_eg"), TextDirection::RightToLeft);
        assert_eq!(direction(""), TextDirection::LeftToRight);
    }

    #[test]
    fn preview_wraps_by_character_not_by_byte() {
        let chinese = "汉".repeat(25);
        // 25 code points, 75 UTF-8 bytes: a byte-based wrap would give 8 lines of 10 bytes.
        assert_eq!(chinese.chars().count(), 25);
        assert_eq!(chinese.len(), 75);

        let mut resources = Resources::new();
        resources
            .load_json(&format!(
                r#"{{"language":"en","translations":{{"sample":{{"message":"{chinese}"}}}}}}"#
            ))
            .expect("the catalogue is well formed");
        resources.select("en");

        let preview = resources.preview("sample", None, 1, 10);
        assert_eq!(preview.text, chinese);
        assert_eq!(preview.characters, 25);
        assert!(!preview.truncated);
        assert_eq!(preview.lines.len(), 3);
        assert_eq!(preview.lines[0], "汉".repeat(10));
        assert_eq!(preview.lines[1], "汉".repeat(10));
        assert_eq!(preview.lines[2], "汉".repeat(5));

        let unwrapped = resources.preview("sample", None, 1, 0);
        assert_eq!(unwrapped.lines.len(), 1);
        assert_eq!(unwrapped.lines[0], chinese);
        assert_eq!(unwrapped.characters, 25);
        assert!(!unwrapped.truncated);
    }

    #[test]
    fn preview_truncates_only_at_the_preview_budget() {
        let long = "字".repeat(MAX_PREVIEW_CHARS + 1);
        let mut resources = Resources::new();
        resources
            .load_json(&format!(
                r#"{{"language":"en","translations":{{"long":{{"message":"{long}"}}}}}}"#
            ))
            .expect("the catalogue is well formed");
        resources.select("en");

        let preview = resources.preview("long", None, 1, 100);
        assert!(preview.truncated);
        assert_eq!(preview.characters, MAX_PREVIEW_CHARS + 1);
        assert_eq!(preview.text.chars().count(), MAX_PREVIEW_CHARS);
        assert_eq!(preview.lines.len(), MAX_PREVIEW_CHARS.div_ceil(100));
    }

    #[test]
    fn reload_of_an_injected_catalogue_returns_the_real_reason() {
        let mut resources = Resources::new();
        resources
            .load_json(
                r#"{
                    "language": "de",
                    "translations": {
                        "hallo": {
                            "message": "Hallo"
                        }
                    }
                }"#,
            )
            .expect("the German catalogue is well formed");

        // `load_json` uses `inject_translations`, which registers no path, so the manager cannot
        // re-read anything. The error is the manager's own reason, not a fabricated success.
        let reason = resources
            .reload("de")
            .expect_err("an injected catalogue has no file to reload");
        assert!(
            reason.contains("no translation file is registered"),
            "unexpected reload reason: {reason}"
        );
        // The failed reload must not have discarded the catalogue that was already loaded.
        assert_eq!(resources.translate("hallo", None, 1), "hallo");
        resources.select("de");
        assert_eq!(resources.translate("hallo", None, 1), "Hallo");
    }

    #[test]
    fn a_catalogue_loaded_from_a_file_can_be_reloaded_after_the_file_changes() {
        let directory = std::env::temp_dir().join(format!(
            "rustage-target-i18n-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        // The manager identifies a language by file name, so the file is `de.json`.
        let path = directory.join("de.json");
        std::fs::write(
            &path,
            r#"{"language":"de","translations":{"hallo":{"message":"Hallo"}}}"#,
        )
        .expect("a catalogue file");

        let mut resources = Resources::new();
        resources.load_file(&path).expect("a file-backed catalogue");
        resources.select("de");
        assert_eq!(resources.translate("hallo", None, 1), "Hallo");

        // The manager registered the path, so reload re-reads the file and the next lookup sees it.
        std::fs::write(
            &path,
            r#"{"language":"de","translations":{"hallo":{"message":"Guten Tag"}}}"#,
        )
        .expect("an edited catalogue file");
        resources.reload("de").expect("a file-backed reload");
        assert_eq!(resources.translate("hallo", None, 1), "Guten Tag");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&directory);
    }

    #[test]
    fn malformed_catalogues_are_refused_with_a_specific_reason() {
        let cases: [(&str, &str); 8] = [
            (r#"{"translations":{}}"#, "missing the \"language\" member"),
            (
                r#"{"language":"de"}"#,
                "missing the \"translations\" member",
            ),
            (
                r#"{"language":"de","translations":[]}"#,
                "\"translations\" member that is not an object",
            ),
            (
                r#"{"language":"de","translations":{"k":{}}}"#,
                "missing the \"message\" member",
            ),
            (
                r#"{"language":"de","translations":{"k":{"message":"M","plural":{"eins":"E"}}}}"#,
                "plural category \"eins\"",
            ),
            (
                r#"{"language":"de","translations":{"k":{"message":"M","plural":{"1":2}}}}"#,
                "plural form 1",
            ),
            (
                r#"{"language":"de","translations":{"k":{"message":"M","plural":{"1":"E","01":"Z"}}}}"#,
                "repeats category 1",
            ),
            (
                r#"{"language":"de","translations":{"k":{"message":"M","context":3}}}"#,
                "\"context\" member that is not a string",
            ),
        ];

        for (text, expected) in cases {
            let mut resources = Resources::new();
            let reason = resources
                .load_json(text)
                .expect_err("the catalogue is malformed");
            assert!(
                reason.contains(expected),
                "reason {reason:?} did not contain {expected:?}"
            );
            // A refused load leaves no partial index behind.
            assert_eq!(resources.audit("de").extra_count(), 0);
        }
    }
}
