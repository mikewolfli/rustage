// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Service logs and the diagnostic bundle (roadmap task T22.10).
//!
//! A support bundle is only safe if three things are true, and this module keeps them separate:
//!
//! * **What it contains is knowable before it leaves.** [`DiagnosticBundle::preview`] lists every
//!   section with its line and byte count and says the content is redacted, so a person approves a
//!   manifest rather than a surprise.
//! * **Content is redacted, not "probably fine".** [`redact`] removes absolute paths, e-mail
//!   addresses, values of secret-looking keys and long opaque strings. The rules are stated here and
//!   tested; a section is never written unredacted.
//! * **Writing and sending need a permit.** [`DiagnosticBundle::write`] refuses a bundle without an
//!   [`ExportPermit`], and a permit is minted only by [`DiagnosticBundle::authorize_local`] (a
//!   `FileWrite` decision) or [`DiagnosticBundle::authorize_external`] (a `Network` decision, which
//!   this host refuses while no project network policy exists). There is no external transport in
//!   this build, so an external permit can be checked but never used to send anything.
//!
//! [`ServiceLog`] is the language/debug service log the bundle includes: entries carry their service
//! so a failure can be attributed ("the language server exited" is not the debug adapter's fault),
//! and its rendered form is redacted the same way as the bundle.

#![allow(dead_code)]

use std::path::Path;

use crate::contracts::permission::{EffectClass, Principal};
use crate::host::log::Level;
use crate::host::permission::{PermissionService, Scope};

/// One service log line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    /// The service it came from, for example `language` or `debug`.
    pub service: String,
    /// How serious it is.
    pub level: Level,
    /// What it says.
    pub message: String,
}

/// The language/debug service log.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct ServiceLog {
    entries: Vec<Entry>,
}

impl ServiceLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one line.
    pub fn push(&mut self, service: impl Into<String>, level: Level, message: impl Into<String>) {
        self.entries.push(Entry {
            service: service.into(),
            level,
            message: message.into(),
        });
    }

    /// How many lines were recorded.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every line.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// One service's lines, in order.
    pub fn for_service(&self, service: &str) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.service == service)
            .collect()
    }

    /// The services that logged something, in order of first appearance.
    pub fn services(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for entry in &self.entries {
            if !names.contains(&entry.service.as_str()) {
                names.push(entry.service.as_str());
            }
        }
        names
    }

    /// The log in one line per entry, redacted.
    pub fn render(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| {
                format!(
                    "{} {}: {}",
                    entry.level.name(),
                    entry.service,
                    redact(&entry.message)
                )
            })
            .collect()
    }
}

/// One section of a bundle.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Section {
    /// Its name, as the preview shows it.
    pub name: String,
    /// Its lines, unredacted in memory; only the redacted form is written.
    pub lines: Vec<String>,
}

/// A support bundle assembled from host facts.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct DiagnosticBundle {
    sections: Vec<Section>,
}

impl DiagnosticBundle {
    /// An empty bundle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a section, replacing one of the same name.
    pub fn section(&mut self, name: impl Into<String>, lines: impl IntoIterator<Item = String>) {
        let name = name.into();
        let lines: Vec<String> = lines.into_iter().collect();
        match self
            .sections
            .iter_mut()
            .find(|section| section.name == name)
        {
            Some(section) => section.lines = lines,
            None => self.sections.push(Section { name, lines }),
        }
    }

    /// The sections in the order they were added.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// How many lines the bundle holds.
    pub fn line_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.lines.len())
            .sum()
    }

    /// What would be written: each section's name, line count and redacted byte count.
    pub fn preview(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "diagnostic bundle: {} section(s), {} line(s), redacted",
            self.sections.len(),
            self.line_count()
        )];
        for section in &self.sections {
            let bytes: usize = section
                .lines
                .iter()
                .map(|line| redact(line).len() + 1)
                .sum();
            lines.push(format!(
                "  {}: {} line(s), {} redacted byte(s)",
                section.name,
                section.lines.len(),
                bytes
            ));
        }
        lines
    }

    /// The redacted bundle text.
    pub fn redacted(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            out.push_str(&format!("## {}\n", section.name));
            for line in &section.lines {
                out.push_str(&redact(line));
                out.push('\n');
            }
        }
        out
    }

    /// Authorizes writing the bundle to a local file.
    pub fn authorize_local(&self, service: &PermissionService) -> Result<ExportPermit, String> {
        let permit = service
            .permit(EffectClass::FileWrite, Scope::Session, Principal::Human)
            .map_err(|refusal| {
                format!(
                    "writing a diagnostic bundle is not authorized: {}",
                    refusal.message()
                )
            })?;
        Ok(ExportPermit {
            target: ExportTarget::Local,
            evidence: permit.evidence(),
        })
    }

    /// Checks whether sending the bundle to `target` is authorized.
    ///
    /// The answer is a permit on success; in this build it is always a refusal, because a network
    /// request needs a project network policy this command cannot invent. That refusal **is** the
    /// safety property the acceptance line asks for: nothing leaves without an authorization that
    /// exists nowhere yet.
    pub fn authorize_external(
        &self,
        service: &PermissionService,
        target: &str,
    ) -> Result<ExportPermit, String> {
        let permit = service
            .permit(EffectClass::Network, Scope::Session, Principal::Human)
            .map_err(|refusal| {
                format!(
                    "sending a diagnostic bundle to {target:?} is not authorized: {}",
                    refusal.message()
                )
            })?;
        Ok(ExportPermit {
            target: ExportTarget::External(target.to_owned()),
            evidence: permit.evidence(),
        })
    }

    /// Writes the redacted bundle, given a permit.
    ///
    /// An external permit cannot write anything here: there is no transport in this build, and
    /// pretending to have sent a bundle would be worse than the refusal.
    pub fn write(&self, permit: &ExportPermit, path: &Path) -> Result<usize, String> {
        if !matches!(permit.target, ExportTarget::Local) {
            return Err(
                "this build has no external transport; nothing was sent and nothing was written"
                    .to_owned(),
            );
        }
        let text = self.redacted();
        std::fs::write(path, &text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
        Ok(text.len())
    }
}

/// Where a bundle is going.
#[derive(Clone, PartialEq, Eq, Debug)]
enum ExportTarget {
    Local,
    External(String),
}

/// Proof that a bundle's destination was authorized.
///
/// The fields are private and the type has no public constructor, so the only way to obtain one is
/// through [`DiagnosticBundle::authorize_local`] or [`DiagnosticBundle::authorize_external`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExportPermit {
    target: ExportTarget,
    evidence: String,
}

impl ExportPermit {
    /// The authorization record this permit came from.
    pub fn evidence(&self) -> &str {
        &self.evidence
    }

    /// Whether it is a local write.
    pub fn is_local(&self) -> bool {
        matches!(self.target, ExportTarget::Local)
    }

    /// The destination, when it is external.
    pub fn target(&self) -> Option<&str> {
        match &self.target {
            ExportTarget::Local => None,
            ExportTarget::External(target) => Some(target),
        }
    }
}

/// Removes the things a support bundle must not carry.
///
/// The rules are deliberately coarse, because a redactor that tries to be clever leaves the one
/// secret it did not recognise:
///
/// 1. an absolute path (a token starting with `/` and containing another `/`, or a `X:\` drive path)
///    becomes `<path>`;
/// 2. an e-mail address becomes `<email>`;
/// 3. the value after a secret-looking key (`token=`, `password:`, `api_key=`, …) becomes
///    `<redacted>`, keeping the key;
/// 4. a long opaque token (40 or more characters with no whitespace) becomes `<redacted>`, which
///    covers bearer tokens, digests and keys that were pasted without a key name.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, tail) = rest.split_at(end);
        out.push_str(&redact_word(word));
        rest = tail;
        if let Some(separator) = rest.chars().next() {
            out.push(separator);
            rest = &rest[separator.len_utf8()..];
        }
    }
    out
}

fn redact_word(word: &str) -> String {
    if word.is_empty() {
        return String::new();
    }
    if is_path(word) {
        return "<path>".to_owned();
    }
    if is_email(word) {
        return "<email>".to_owned();
    }
    if let Some((key, _)) = word.split_once('=')
        && is_secret_key(key)
    {
        return format!("{key}=<redacted>");
    }
    if let Some((key, _)) = word.split_once(':')
        && is_secret_key(key)
    {
        return format!("{key}:<redacted>");
    }
    if word.chars().count() >= 40 && !word.chars().any(|character| character.is_whitespace()) {
        return "<redacted>".to_owned();
    }
    word.to_owned()
}

fn is_path(word: &str) -> bool {
    if word.starts_with('/') && word[1..].contains('/') {
        return true;
    }
    let mut characters = word.chars();
    matches!(
        (characters.next(), characters.next()),
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic()
    ) && word.contains('\\')
}

fn is_email(word: &str) -> bool {
    let Some((local, domain)) = word.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !local.contains('/')
        && !domain.contains('/')
}

fn is_secret_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "token"
            | "secret"
            | "password"
            | "passwd"
            | "api_key"
            | "apikey"
            | "authorization"
            | "credential"
            | "private_key"
    )
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticBundle, Level, ServiceLog, redact};
    use crate::host::permission::PermissionService;

    #[test]
    fn paths_emails_and_secret_values_are_removed() {
        assert_eq!(
            redact("loading /Users/me/project/src/main.rs now"),
            "loading <path> now"
        );
        assert_eq!(redact("mail me at dev@example.com"), "mail me at <email>");
        assert_eq!(redact("token=abc123 kept"), "token=<redacted> kept");
        assert_eq!(
            redact("Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH"),
            "Authorization:<redacted> Bearer <redacted>"
        );
        assert_eq!(
            redact("digest 0123456789abcdef0123456789abcdef01234567"),
            "digest <redacted>"
        );
        assert_eq!(redact("ordinary words stay"), "ordinary words stay");
    }

    #[test]
    fn a_bundle_previews_before_it_writes_and_never_writes_unredacted() {
        let mut bundle = DiagnosticBundle::new();
        bundle.section(
            "host",
            ["rustage 0.1.0".to_owned(), "macos aarch64".to_owned()],
        );
        bundle.section(
            "log",
            ["ERROR language: cannot open /Users/me/project.log".to_owned()],
        );
        let preview = bundle.preview();
        assert!(preview[0].contains("2 section(s)"), "{preview:?}");
        assert!(preview[1].contains("host: 2 line(s)"), "{preview:?}");
        assert!(preview[1].contains("redacted"), "{preview:?}");

        let text = bundle.redacted();
        assert!(!text.contains("/Users/me"), "{text}");
        assert!(text.contains("<path>"), "{text}");
    }

    #[test]
    fn writing_needs_a_local_permit_and_an_external_send_is_refused() {
        let service = PermissionService::new();
        let mut bundle = DiagnosticBundle::new();
        bundle.section("host", ["rustage 0.1.0".to_owned()]);

        let permit = bundle.authorize_local(&service).expect("a local write");
        assert!(permit.is_local());
        assert!(permit.evidence().contains("file-write"));

        let error = bundle
            .authorize_external(&service, "https://example.com/upload")
            .expect_err("no network policy exists");
        assert!(error.contains("not authorized"), "{error}");
        assert!(error.contains("network"), "{error}");

        let directory = std::env::temp_dir().join(format!(
            "rustage-bundle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("a directory");
        let path = directory.join("bundle.txt");
        let bytes = bundle.write(&permit, &path).expect("a write");
        let written = std::fs::read_to_string(&path).expect("the file");
        assert_eq!(written.len(), bytes);
        assert!(written.contains("rustage 0.1.0"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_service_log_keeps_services_apart_and_renders_redacted() {
        let mut log = ServiceLog::new();
        log.push("language", Level::Error, "cannot start at /tmp/lsp");
        log.push("debug", Level::Info, "adapter ready");
        log.push("language", Level::Warn, "retrying");
        assert_eq!(log.len(), 3);
        assert_eq!(log.services(), vec!["language", "debug"]);
        assert_eq!(log.for_service("language").len(), 2);
        let lines = log.render();
        assert_eq!(lines[0], "ERROR language: cannot start at <path>");
        assert_eq!(lines[1], "INFO debug: adapter ready");
        assert!(lines[2].ends_with("retrying"));
    }
}
