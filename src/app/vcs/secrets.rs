// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The secret scan a commit goes through (roadmap task T20.03).
//!
//! The acceptance line is *"a commit does not include unauthorized changes or secrets"*. The
//! unauthorized-change half is the preview's staged-only list; this module is the secret half.
//!
//! * **The patterns are named and few.** A finding says which rule fired, so a person can check it
//!   instead of trusting a red word. The rules cover the shapes that are unambiguous on their own —
//!   a private-key header, an AWS access key id, a GitHub or Slack token, an `sk-` key — plus one
//!   conservative `name = value` rule for the common secret field names.
//! * **A placeholder is not a secret.** `$TOKEN`, `<your-token>`, `{{token}}`, `changeme`, `xxx`,
//!   `example`, `REDACTED` and a value of one repeated character are not findings: a scanner that
//!   cries wolf on documentation gets turned off, and then it protects nothing.
//! * **One finding per line at most.** The first rule that matches a line names it; a line holding
//!   two shapes is reported once, because the fix is the same line.
//! * **Binary content is not scanned here.** A caller that cannot decode a file as UTF-8 reports it
//!   as skipped rather than as clean — "not looked at" is not "no secret".

/// What kind of secret a line looks like.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A PEM private-key header.
    PrivateKey,
    /// An AWS access key id (`AKIA…`).
    AwsAccessKey,
    /// A GitHub token (`ghp_…`, `github_pat_…`).
    GitHubToken,
    /// An `sk-…` API key.
    ApiKey,
    /// A Slack token (`xoxb-…` and the other `xox` prefixes).
    SlackToken,
    /// A secret-named field assigned a value.
    SecretAssignment,
}

impl Kind {
    /// The kind's name, as a finding writes it.
    pub fn name(self) -> &'static str {
        match self {
            Self::PrivateKey => "private-key",
            Self::AwsAccessKey => "aws-access-key",
            Self::GitHubToken => "github-token",
            Self::ApiKey => "api-key",
            Self::SlackToken => "slack-token",
            Self::SecretAssignment => "secret-assignment",
        }
    }
}

/// One finding.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Secret {
    /// The path the line is in.
    pub path: String,
    /// The line number, 1-based.
    pub line: usize,
    /// Which rule fired.
    pub kind: Kind,
}

impl Secret {
    /// The finding in one line.
    pub fn render(&self) -> String {
        format!("{}:{} {}", self.path, self.line, self.kind.name())
    }
}

/// The field names the assignment rule looks for.
const SECRET_NAMES: [&str; 8] = [
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "access_key",
    "auth",
];

/// Values that are documentation rather than secrets.
const PLACEHOLDERS: [&str; 7] = [
    "changeme",
    "example",
    "redacted",
    "placeholder",
    "your",
    "none",
    "null",
];

/// The findings in one text, in line order.
pub fn scan(path: &str, text: &str) -> Vec<Secret> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(kind) = classify(line) {
            found.push(Secret {
                path: path.to_owned(),
                line: index + 1,
                kind,
            });
        }
    }
    found
}

/// Which rule one line matches, if any.
fn classify(line: &str) -> Option<Kind> {
    if line.contains("-----BEGIN ") && line.contains("PRIVATE KEY-----") {
        return Some(Kind::PrivateKey);
    }
    for token in tokens(line) {
        if is_aws_access_key(token) {
            return Some(Kind::AwsAccessKey);
        }
        if is_github_token(token) {
            return Some(Kind::GitHubToken);
        }
        if is_sk_key(token) {
            return Some(Kind::ApiKey);
        }
        if is_slack_token(token) {
            return Some(Kind::SlackToken);
        }
    }
    if assignment_holds_a_secret(line) {
        return Some(Kind::SecretAssignment);
    }
    None
}

/// The runs of characters a token can be made of.
fn tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|character: char| {
        !(character.is_ascii_alphanumeric() || character == '_' || character == '-')
    })
    .filter(|token| !token.is_empty())
}

/// `AKIA` followed by sixteen upper-case letters or digits.
fn is_aws_access_key(token: &str) -> bool {
    let Some(rest) = token.strip_prefix("AKIA") else {
        return false;
    };
    rest.len() == 16
        && rest
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
}

/// A GitHub personal or fine-grained token.
fn is_github_token(token: &str) -> bool {
    if let Some(rest) = token.strip_prefix("ghp_") {
        return rest.len() >= 20;
    }
    if let Some(rest) = token.strip_prefix("github_pat_") {
        return rest.len() >= 20;
    }
    false
}

/// An `sk-` key long enough to be a credential rather than prose.
fn is_sk_key(token: &str) -> bool {
    match token.strip_prefix("sk-") {
        Some(rest) => {
            rest.len() >= 20
                && rest
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }
        None => false,
    }
}

/// A Slack token.
fn is_slack_token(token: &str) -> bool {
    for prefix in ["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-"] {
        if let Some(rest) = token.strip_prefix(prefix) {
            return rest.len() >= 10;
        }
    }
    false
}

/// A secret-named field assigned a real value.
fn assignment_holds_a_secret(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    for name in SECRET_NAMES {
        let mut start = 0;
        while let Some(offset) = lower[start..].find(name) {
            let position = start + offset;
            let after = &line[position + name.len()..];
            // The name must be the field, not the end of a longer identifier.
            let before_is_word = line[..position]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_alphanumeric() || character == '_');
            if before_is_word {
                start = position + 1;
                continue;
            }
            // A field name is often quoted (`"api_key": ...`), so a closing quote may sit
            // between the name and its separator.
            let trimmed = after
                .trim_start()
                .trim_start_matches(['"', '\'', '`'])
                .trim_start();
            let value = match trimmed.strip_prefix('=') {
                Some(rest) => rest,
                None => match trimmed.strip_prefix(':') {
                    Some(rest) => rest,
                    None => {
                        start = position + 1;
                        continue;
                    }
                },
            };
            if value_is_secret(value) {
                return true;
            }
            start = position + 1;
        }
    }
    false
}

/// Whether the text after `name =` or `name:` is a value rather than a placeholder.
fn value_is_secret(value: &str) -> bool {
    let value = value.trim();
    let value = value.trim_matches(|character| {
        matches!(character, '"' | '\'' | '`' | ',' | ';' | ')' | ']' | '}')
    });
    if value.len() < 8 {
        return false;
    }
    // A reference is not a value: `$TOKEN`, `${TOKEN}`, `<token>`, `{{ token }}`.
    if value.starts_with('$') || value.starts_with('<') || value.starts_with('{') {
        return false;
    }
    // A call is code, not a credential: `read_token()`, `get("x")`, `env[...]`.
    if value.contains('(') || value.contains('[') {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if PLACEHOLDERS
        .iter()
        .any(|placeholder| lower.contains(placeholder))
    {
        return false;
    }
    // One repeated character is a redaction, not a secret.
    let mut characters = value.chars();
    if let Some(first) = characters.next()
        && characters.all(|character| character == first)
    {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{Kind, Secret, scan};

    fn kinds(text: &str) -> Vec<Kind> {
        scan("f", text)
            .into_iter()
            .map(|secret| secret.kind)
            .collect()
    }

    #[test]
    fn a_private_key_header_is_named() {
        assert_eq!(
            kinds("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n"),
            vec![Kind::PrivateKey]
        );
    }

    #[test]
    fn each_credential_shape_is_named() {
        assert_eq!(
            kinds("key = AKIAIOSFODNN7EXAMPLE"),
            vec![Kind::AwsAccessKey]
        );
        assert_eq!(
            kinds("token: ghp_1234567890abcdefghijklmnopqrstuvwx"),
            vec![Kind::GitHubToken]
        );
        assert_eq!(
            kinds("OPENAI=sk-abcdefghijklmnopqrstuvwx"),
            vec![Kind::ApiKey]
        );
        assert_eq!(
            kinds("url = https://hooks.slack.com/xoxb-1234567890-abcdef"),
            vec![Kind::SlackToken]
        );
    }

    #[test]
    fn a_secret_named_field_holding_a_value_is_a_finding() {
        assert_eq!(
            kinds("password = hunter2hunter2"),
            vec![Kind::SecretAssignment]
        );
        assert_eq!(
            kinds("  \"api_key\": \"a8f5f167f44f4964e6c998dee827110c\""),
            vec![Kind::SecretAssignment]
        );
        assert_eq!(kinds("AUTH=abcdefghijkl"), vec![Kind::SecretAssignment]);
    }

    #[test]
    fn a_placeholder_is_not_a_secret() {
        for line in [
            "password = $DB_PASSWORD",
            "token: ${GITHUB_TOKEN}",
            "secret = <your-secret>",
            "api_key = {{ api_key }}",
            "password = changeme",
            "token = example-token-for-docs",
            "secret = xxxxxxxx",
            "password =",
        ] {
            assert!(scan("f", line).is_empty(), "{line:?} was reported");
        }
    }

    #[test]
    fn the_name_must_be_a_field_rather_than_the_end_of_a_word() {
        // `subtokens` and `mytoken` are not the field `token`.
        assert!(scan("f", "subtokens = abcdefghijkl").is_empty());
        assert!(scan("f", "mytoken = abcdefghijkl").is_empty());
    }

    #[test]
    fn a_line_is_reported_once_and_a_finding_keeps_its_line_number() {
        let text = "first\npassword = hunter2hunter2 AKIAIOSFODNN7EXAMPLE\nthird\n";
        let found = scan("a.rs", text);
        assert_eq!(found.len(), 1, "{found:?}");
        // The credential shapes are checked before the assignment rule, so the more specific
        // finding names the line.
        assert_eq!(
            found[0],
            Secret {
                path: "a.rs".to_owned(),
                line: 2,
                kind: Kind::AwsAccessKey,
            }
        );
        assert_eq!(found[0].render(), "a.rs:2 aws-access-key");
    }

    #[test]
    fn ordinary_text_has_no_findings() {
        assert!(scan("f", "let token = read_token()?;\nlet api_key = 3;\n").is_empty());
    }
}
