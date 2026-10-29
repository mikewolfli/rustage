// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The generated application's language resources (task T24.04).
//!
//! This is separate from `--ui-language`: that flag chooses the *designer's* interface language,
//! while these operations load and inspect the catalogues a generated application ships. Keeping
//! them apart is the roadmap's own boundary — translating the host must never be presented as the
//! target application being translated.

use super::*;

/// How to load and inspect a generated application's translations (task T24.04).
#[derive(Debug, PartialEq, Eq)]
pub struct TargetI18nOptions {
    /// The operations: `load:<file>`, `language:<tag>`, `context:<text>` (empty clears it),
    /// `count:<n>`, `translate:<key>`, `audit`, `direction:<tag>`, `preview:<key>:<max-chars>`,
    /// `reload`, `show`.
    pub ops: Vec<String>,
}

impl TargetI18nOptions {
    /// Options with no catalogue loaded.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for TargetI18nOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_target_i18n(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--target-i18n" => {
            let mut options = TargetI18nOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            Some(Ok(Command::TargetI18n(options)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_target_i18n;
    use crate::cli::command::{Command, TargetI18nOptions};

    fn parse(values: &[&str]) -> Result<Command, String> {
        let mut arguments = values
            .iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
            .into_iter();
        let Some(flag) = arguments.next() else {
            return Err("no flag".to_owned());
        };
        let text = flag.to_string_lossy().into_owned();
        match parse_target_i18n(&text, flag, &mut arguments) {
            Some(Ok(command)) => Ok(command),
            Some(Err(error)) => Err(error.to_string()),
            None => Err("not a target-i18n flag".to_owned()),
        }
    }

    #[test]
    fn every_operation_is_collected_in_order() {
        let command =
            parse(&["--target-i18n", "load:a.json", "language:de", "show"]).expect("a command");
        assert_eq!(
            command,
            Command::TargetI18n(TargetI18nOptions {
                ops: vec![
                    "load:a.json".to_owned(),
                    "language:de".to_owned(),
                    "show".to_owned()
                ],
            })
        );
    }

    #[test]
    fn a_following_flag_is_not_swallowed() {
        assert!(parse(&["--target-i18n", "show", "--help"]).is_err());
    }
}
