// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Host-side language tooling: key extraction and pseudo-localization (task T24.10).
//!
//! The designer's own catalogue is a data file, not a Rust table (blue21 L-1). Two checks keep that
//! data honest while people edit it:
//!
//! * **Key extraction** reads the window's own source for `tr("…")`/`tr_args("…")` calls and names
//!   the catalog keys the code actually asks for. A used key that the vocabulary does not identify
//!   is a label that will paint as a raw identifier; the audit reports it rather than waiting for a
//!   person to notice.
//! * **Pseudo-localization** rewrites every message into a longer, accented, bracketed form with the
//!   `{name}` placeholders kept verbatim. Pasting the pseudo pack in place of the real one is how a
//!   layout is checked for clipped labels and hard-coded widths without speaking every language.
//!
//! Neither function reads or writes a document, a resource identity or a generated artifact; they
//! operate on the interface catalogue only.

use std::collections::BTreeSet;

use super::catalog::Pack;
use super::vocabulary::Entry;

/// The prefixes a catalog key uses.
const KEY_PREFIXES: [&str; 5] = ["ui.", "cmd.", "menu.", "diag.", "shell."];

/// Every catalog key the source names, sorted and de-duplicated.
///
/// Two reads are combined. The first normalises whitespace and then finds the string literal a
/// `tr(…)`/`tr_args(…)` call takes, so a multi-line call is read the same way as a one-line one. The
/// second collects `ui.*` literals wherever they appear, because the window passes many of them
/// through variables (`tr(key)` in a loop) and a call-only scan would call those unused.
pub fn extract_keys(source: &str) -> Vec<String> {
    let mut keys = BTreeSet::new();
    let compact: String = source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    for marker in ["tr(\"", "tr_args(\""] {
        let mut rest = compact.as_str();
        while let Some(position) = rest.find(marker) {
            let after = &rest[position + marker.len()..];
            let Some(end) = after.find('"') else {
                break;
            };
            let key = &after[..end];
            if looks_like_key(key) {
                keys.insert(key.to_owned());
            }
            rest = &after[end..];
        }
    }
    for piece in source.split('"').skip(1).step_by(2) {
        if looks_like_window_key(piece) {
            keys.insert(piece.to_owned());
        }
    }
    keys.into_iter().collect()
}

/// Whether a literal is shaped like a catalog key rather than a human sentence.
fn looks_like_key(key: &str) -> bool {
    KEY_PREFIXES.iter().any(|prefix| key.starts_with(prefix))
        && !key.contains(char::is_whitespace)
        && key.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

/// Whether a literal is a window-scoped key: at least three lower-case segments.
///
/// The extra structure is what keeps file names (`ui.rs`), prefix fragments (`ui.`) and Windows
/// extension lists (`cmd.BAT`) out of the audit.
fn looks_like_window_key(key: &str) -> bool {
    if !key.starts_with("ui.") {
        return false;
    }
    let parts: Vec<&str> = key.split('.').collect();
    parts.len() >= 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || matches!(character, '-' | '_')
                })
        })
}

/// What an audit found.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Audit {
    /// Keys the source asks for that the vocabulary does not identify.
    pub missing: Vec<String>,
    /// Window keys the vocabulary identifies that no scanned call asks for.
    pub unused: Vec<String>,
}

impl Audit {
    /// Whether every used key is identified.
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Reads every Rust source under `directory` and collects the keys it names.
///
/// `target/`, `.git/` and `node_modules/` are skipped because they are build output, history and
/// dependencies rather than code. `vocabulary.rs` is skipped because it *defines* the key set;
/// counting its literals as uses would make every key look asked for.
pub fn collect_keys(directory: &std::path::Path) -> Result<(Vec<String>, usize), String> {
    let mut used = Vec::new();
    let mut files = 0usize;
    walk(directory, &mut used, &mut files)?;
    Ok((used, files))
}

fn walk(
    directory: &std::path::Path,
    used: &mut Vec<String>,
    files: &mut usize,
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("could not read an entry: {error}"))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(name.as_ref(), "target" | ".git" | "node_modules") {
                continue;
            }
            walk(&path, used, files)?;
            continue;
        }
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        if name == "vocabulary.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        *files += 1;
        used.extend(extract_keys(&text));
    }
    Ok(())
}

/// Compares the keys the source uses against the identified vocabulary.
///
/// Only window-scoped keys (`ui.*`) are reported as unused: command, menu, diagnostic and shell keys
/// are looked up through other doors, and calling them "unused" would be a false finding.
pub fn audit(used: &[String], identified: &[Entry]) -> Audit {
    let known: BTreeSet<&str> = identified.iter().map(|entry| entry.key.as_str()).collect();
    let used_set: BTreeSet<&str> = used.iter().map(String::as_str).collect();
    let missing = used
        .iter()
        .filter(|key| !known.contains(key.as_str()))
        .cloned()
        .collect();
    let unused = identified
        .iter()
        .filter(|entry| entry.key.starts_with("ui."))
        .filter(|entry| !used_set.contains(entry.key.as_str()))
        .map(|entry| entry.key.clone())
        .collect();
    Audit { missing, unused }
}

/// Rewrites every message into a pseudo-localized form.
///
/// The braces of a `{name}` placeholder are copied through with their contents untouched, so
/// [`crate::app::locale::catalog::Catalog::fill`] still works on the pseudo pack. ASCII letters are
/// accented and the message is bracketed, which makes a clipped label and an underestimated width
/// visible without translating anything.
pub fn pseudo_pack(pack: &Pack) -> Pack {
    let mut pseudo = Pack::new(pack.language());
    for (key, text) in pack.iter() {
        pseudo.insert(key, pseudo_text(text));
    }
    pseudo
}

/// The pseudo-localized form of one message.
pub fn pseudo_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    out.push('⟦');
    let mut in_placeholder = false;
    for character in text.chars() {
        match character {
            '{' => {
                in_placeholder = true;
                out.push(character);
            }
            '}' => {
                in_placeholder = false;
                out.push(character);
            }
            _ if in_placeholder => out.push(character),
            'a'..='z' => out.push(accent_lower(character)),
            'A'..='Z' => out.push(accent_upper(character)),
            other => out.push(other),
        }
    }
    out.push('⟧');
    out
}

fn accent_lower(character: char) -> char {
    match character {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ó',
        'u' => 'ú',
        'y' => 'ý',
        'c' => 'ç',
        'n' => 'ñ',
        's' => 'š',
        'z' => 'ž',
        other => other,
    }
}

fn accent_upper(character: char) -> char {
    match character {
        'A' => 'Á',
        'E' => 'É',
        'I' => 'Í',
        'O' => 'Ó',
        'U' => 'Ú',
        'Y' => 'Ý',
        'C' => 'Ç',
        'N' => 'Ñ',
        'S' => 'Š',
        'Z' => 'Ž',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Audit, audit, extract_keys, pseudo_pack, pseudo_text};
    use crate::app::locale::catalog::Pack;
    use crate::app::locale::vocabulary::{Entry, Source};
    use crate::host::locale::Language;

    #[test]
    fn the_host_source_uses_only_identified_keys() {
        // A regression gate for the catalog: a `tr` key that is not in the vocabulary paints as a
        // raw identifier, and the audit is the cheap way to catch it before a window shows it.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let (used, files) = super::collect_keys(&root).expect("the source tree is readable");
        assert!(files > 0, "no Rust source was scanned");
        let audit = audit(&used, &crate::app::locale::vocabulary::entries());
        assert!(
            audit.is_clean(),
            "keys used but not identified: {:?}",
            audit.missing
        );
    }

    #[test]
    fn a_used_key_is_extracted_from_one_line_and_multi_line_calls() {
        let source = r#"
            let a = tr("ui.panel.pages");
            let b = tr_args(
                "ui.error.not-applied",
                &[("reason", reason)],
            );
            let c = tr("not a key");
        "#;
        assert_eq!(
            extract_keys(source),
            vec![
                "ui.error.not-applied".to_owned(),
                "ui.panel.pages".to_owned()
            ]
        );
    }

    #[test]
    fn a_pseudo_message_keeps_every_placeholder_and_changes_the_letters() {
        let text = pseudo_text("there is no command {id}");
        assert!(text.starts_with('⟦'), "{text}");
        assert!(text.ends_with('⟧'), "{text}");
        assert!(text.contains("{id}"), "{text}");
        assert!(!text.contains("command"), "{text}");
    }

    #[test]
    fn a_pseudo_pack_answers_every_key_it_was_given() {
        let mut pack = Pack::new(Language::English);
        pack.insert("ui.status.ready", "ready");
        pack.insert("ui.status.hidden", "hidden {id}");
        let pseudo = pseudo_pack(&pack);
        assert_eq!(pseudo.len(), pack.len());
        assert!(pseudo.get("ui.status.hidden").unwrap().contains("{id}"));
        let read_back = Pack::decode(&pseudo.encode()).expect("the pseudo pack is a language file");
        assert_eq!(read_back, pseudo);
    }

    #[test]
    fn an_identified_key_is_not_reported_missing_and_a_ui_key_is_reported_unused() {
        let identified = vec![
            Entry {
                key: "ui.panel.pages".to_owned(),
                english: "Pages".to_owned(),
                source: Source::Window,
            },
            Entry {
                key: "ui.panel.data".to_owned(),
                english: "Data & State".to_owned(),
                source: Source::Window,
            },
            Entry {
                key: "cmd.file.save.title".to_owned(),
                english: "Save".to_owned(),
                source: Source::Command,
            },
        ];
        let unknown = ["ui", "unknown", "key"].join(".");
        let used = vec!["ui.panel.pages".to_owned(), unknown.clone()];
        let audit = audit(&used, &identified);
        assert_eq!(audit.missing, vec![unknown]);
        assert_eq!(audit.unused, vec!["ui.panel.data".to_owned()]);
        assert!(!audit.is_clean());
        assert_eq!(Audit::default(), Audit::default());
    }
}
