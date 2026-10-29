// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--target-i18n` command: the generated application's language resources (task T24.04).
//!
//! The operations read a library `TranslationFile` shape from a file and answer the questions a
//! designer asks about the target application's translations: what a key resolves to with a plural
//! count and a context, which keys are missing relative to `en`, whether a locale reads right to
//! left, and how long a message is. A reload is attempted through the library manager and its real
//! answer is printed; an embedded catalogue has no file to re-read, and that refusal is the truth
//! rather than a claimed success.

use crate::app::language::target::{self, Resources, Selection};
use crate::cli::TargetI18nOptions;

/// Runs the target-language operations, printing each answer as it is read.
pub(super) fn target_i18n(options: &TargetI18nOptions) -> bool {
    let mut resources = Resources::new();
    let mut language = target::ENGLISH.to_owned();
    let mut context: Option<String> = None;
    let mut count: u32 = 1;

    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "load" => {
                if argument.is_empty() {
                    Err("load needs <file>".to_owned())
                } else {
                    match resources.load_file(std::path::Path::new(argument)) {
                        Ok(loaded) => Ok(vec![format!(
                            "loaded {} ({} key(s), {})",
                            loaded.language,
                            loaded.keys,
                            if loaded.replaced { "replaced" } else { "new" }
                        )]),
                        Err(reason) => Err(reason),
                    }
                }
            }
            "language" => {
                if argument.is_empty() {
                    Err("language needs <tag>".to_owned())
                } else {
                    match resources.select(argument) {
                        Selection::Loaded { language: active } => {
                            language = active.clone();
                            Ok(vec![format!("language {active} (loaded)")])
                        }
                        Selection::FallsBackToEnglish {
                            requested,
                            fallback,
                        } => {
                            language = requested.clone();
                            Ok(vec![format!(
                                "language {requested} (not loaded; lookups fall back to {fallback})"
                            )])
                        }
                        Selection::Unavailable { requested } => Err(format!(
                            "language {requested:?} is not loaded and {} is not loaded either",
                            target::ENGLISH
                        )),
                    }
                }
            }
            "context" => {
                context = if argument.is_empty() {
                    None
                } else {
                    Some(argument.to_owned())
                };
                Ok(vec![format!(
                    "context {}",
                    context.as_deref().unwrap_or("(none)")
                )])
            }
            "count" => match argument.parse::<u32>() {
                Ok(value) => {
                    count = value;
                    Ok(vec![format!("count {value}")])
                }
                Err(_) => Err(format!("{argument:?} is not a count")),
            },
            "translate" => {
                if argument.is_empty() {
                    Err("translate needs <key>".to_owned())
                } else {
                    let text = resources.translate(argument, context.as_deref(), count);
                    Ok(vec![format!("{argument} = {text}")])
                }
            }
            "audit" => {
                let audit = resources.audit(&language);
                let mut lines = vec![format!(
                    "audit {} ({} missing, {} extra)",
                    audit.language,
                    audit.missing_count(),
                    audit.extra_count()
                )];
                for key in &audit.missing {
                    lines.push(format!("missing: {key}"));
                }
                for key in &audit.extra {
                    lines.push(format!("extra: {key}"));
                }
                Ok(lines)
            }
            "direction" => {
                if argument.is_empty() {
                    Err("direction needs <tag>".to_owned())
                } else {
                    let direction = target::direction(argument);
                    Ok(vec![format!(
                        "direction {argument} {}",
                        if direction.is_right_to_left() {
                            "rtl"
                        } else {
                            "ltr"
                        }
                    )])
                }
            }
            "preview" => match argument.split_once(':') {
                Some((key, max)) => match max.parse::<usize>() {
                    Ok(max) => {
                        let preview = resources.preview(key, context.as_deref(), count, max);
                        Ok(vec![
                            format!(
                                "preview {key}: {} char(s), {} line(s), truncated {}",
                                preview.characters,
                                preview.lines.len(),
                                preview.truncated
                            ),
                            format!("text: {}", preview.text),
                        ])
                    }
                    Err(_) => Err(format!("{max:?} is not a character count")),
                },
                None => Err(format!("preview needs <key>:<max-chars>, got {argument:?}")),
            },
            "reload" => match resources.reload(&language) {
                Ok(()) => Ok(vec![format!("reloaded {language}")]),
                Err(reason) => Err(reason),
            },
            "show" => Ok(vec![
                format!("language: {language}"),
                format!("context: {}", context.as_deref().unwrap_or("(none)")),
                format!("count: {count}"),
            ]),
            other => Err(format!(
                "{other:?} is not a target language-resource operation"
            )),
        };
        match step {
            Ok(lines) => {
                for line in lines {
                    println!("{line}");
                }
            }
            Err(reason) => return super::report(reason),
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::target_i18n;
    use crate::cli::TargetI18nOptions;

    fn options(ops: &[&str]) -> TargetI18nOptions {
        TargetI18nOptions {
            ops: ops.iter().map(|op| (*op).to_owned()).collect(),
        }
    }

    #[test]
    fn an_operation_the_command_does_not_know_is_reported() {
        assert!(!target_i18n(&options(&["teleport"])));
    }

    #[test]
    fn a_malformed_preview_or_count_is_reported() {
        assert!(!target_i18n(&options(&["count:many"])));
        assert!(!target_i18n(&options(&["preview:greeting"])));
    }
}
