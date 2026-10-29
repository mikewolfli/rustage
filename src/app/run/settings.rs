// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The `--settings` command: layered settings, migration and keymap import (task F24).

use crate::app::settings::{Keymap, Kind, Layer, Settings};

/// Runs the settings operations, printing each answer as it is read.
pub(super) fn settings(options: &crate::cli::SettingsOptions) -> bool {
    let mut profile = Settings::new();
    let mut keymap = Keymap::default();
    let mut other = Keymap::default();
    for op in &options.ops {
        let (verb, argument) = op.split_once(':').unwrap_or((op.as_str(), ""));
        let step: Result<Vec<String>, String> = match verb {
            "schema" => match argument.split_once(':') {
                Some((key, kind)) => match parse_kind(kind) {
                    Ok(kind) => {
                        profile.schema_key(key, kind);
                        Ok(vec![format!("schema {key}")])
                    }
                    Err(reason) => Err(reason),
                },
                None => Err(format!("schema needs <key>:<kind>, got {argument:?}")),
            },
            "set" => match argument.split_once(':') {
                Some((layer, rest)) => match (Layer::parse(layer), rest.split_once('=')) {
                    (Some(layer), Some((key, value))) => {
                        if let Err(reason) = profile.set(layer, key, value) {
                            return super::report(reason);
                        }
                        Ok(vec![format!("set {key} = {value} ({})", layer.name())])
                    }
                    (None, _) => Err(format!("{layer:?} is not a settings layer")),
                    (_, None) => Err(format!("set needs <layer>:<key>=<value>, got {argument:?}")),
                },
                None => Err(format!("set needs <layer>:<key>=<value>, got {argument:?}")),
            },
            "get" => match profile.effective(argument) {
                Some((value, layer)) => {
                    Ok(vec![format!("{argument} = {value} ({})", layer.name())])
                }
                None => Ok(vec![format!("{argument} = (unset)")]),
            },
            "search" => Ok(profile.search(argument)),
            "reset" => match argument.split_once(':') {
                Some((layer, key)) => match Layer::parse(layer) {
                    Some(layer) => {
                        if let Err(reason) = profile.reset(key, layer) {
                            return super::report(reason);
                        }
                        Ok(vec![format!("reset {key} at {}", layer.name())])
                    }
                    None => Err(format!("{layer:?} is not a settings layer")),
                },
                None => Err(format!("reset needs <layer>:<key>, got {argument:?}")),
            },
            "migrate" => {
                let mut renames = Vec::new();
                for pair in argument.split(',') {
                    match pair.split_once('=') {
                        Some((old, new)) => {
                            renames.push((old.to_owned(), new.to_owned()));
                        }
                        None => return super::report(format!("{pair:?} is not <old>=<new>")),
                    }
                }
                Ok(match profile.migrate(&renames) {
                    Ok(lines) => lines,
                    Err(reason) => return super::report(reason),
                })
            }
            "keymap" => match argument.split_once('=') {
                Some((command, chord)) => {
                    keymap.bind(command, chord);
                    Ok(vec![format!("keymap {command} = {chord}")])
                }
                None => Err(format!("keymap needs <command>=<chord>, got {argument:?}")),
            },
            "keymap-other" => match argument.split_once('=') {
                Some((command, chord)) => {
                    other.bind(command, chord);
                    Ok(vec![format!("other keymap {command} = {chord}")])
                }
                None => Err(format!(
                    "keymap-other needs <command>=<chord>, got {argument:?}"
                )),
            },
            "keymap-diff" => Ok(keymap.import_diff(&other)),
            "keymap-import" => Ok(keymap.import(&other)),
            "import-diff" => match argument.split_once(':') {
                Some((layer, pairs)) => match Layer::parse(layer) {
                    Some(layer) => {
                        let other = match other_profile(&profile, layer, pairs) {
                            Ok(other) => other,
                            Err(reason) => return super::report(reason),
                        };
                        Ok(profile.import_diff(&other))
                    }
                    None => Err(format!("{layer:?} is not a settings layer")),
                },
                None => Err(format!(
                    "import-diff needs <layer>:<key>=<value>[,...], got {argument:?}"
                )),
            },
            "import" => match argument.split_once(':') {
                Some((layer, pairs)) => match Layer::parse(layer) {
                    Some(layer) => {
                        let other = match other_profile(&profile, layer, pairs) {
                            Ok(other) => other,
                            Err(reason) => return super::report(reason),
                        };
                        Ok(match profile.import(&other, layer) {
                            Ok(lines) => lines,
                            Err(reason) => return super::report(reason),
                        })
                    }
                    None => Err(format!("{layer:?} is not a settings layer")),
                },
                None => Err(format!(
                    "import needs <layer>:<key>=<value>[,...], got {argument:?}"
                )),
            },
            "export" => {
                if argument.is_empty() {
                    return super::report(
                        "export needs <file>: the file a person reads when a value is wrong"
                            .to_owned(),
                    );
                }
                let text = profile.encode();
                if let Err(error) = std::fs::write(argument, &text) {
                    return super::report(format!("cannot write {argument}: {error}"));
                }
                // Read the file back, so the report cannot claim an export the file does not hold.
                let read_back = match std::fs::read_to_string(argument) {
                    Ok(text) => text,
                    Err(error) => {
                        return super::report(format!("cannot read {argument}: {error}"));
                    }
                };
                let mut check = profile.empty_like();
                match check.decode(&read_back) {
                    Ok(_) if check == profile => {}
                    Ok(_) => {
                        return super::report(format!(
                            "{argument} does not read back as the exported profile"
                        ));
                    }
                    Err(fault) => {
                        return super::report(format!("{argument}:{fault}"));
                    }
                }
                let count = text.lines().count().saturating_sub(1);
                Ok(vec![format!("exported {count} setting(s) to {argument}")])
            }
            "import-file" => {
                let text = match std::fs::read_to_string(argument) {
                    Ok(text) => text,
                    Err(error) => {
                        return super::report(format!("cannot read {argument}: {error}"));
                    }
                };
                match profile.decode(&text) {
                    Ok(lines) => Ok(lines),
                    Err(fault) => {
                        return super::report(format!("{argument}:{fault}"));
                    }
                }
            }
            "show" => {
                let mut lines = profile.render();
                lines.push(format!("keymap bindings: {}", keymap.bindings.len()));
                Ok(lines)
            }
            other => Err(format!("{other:?} is not a settings operation")),
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

fn other_profile(profile: &Settings, layer: Layer, pairs: &str) -> Result<Settings, String> {
    let mut other = profile.empty_like();
    for pair in pairs.split(',') {
        match pair.split_once('=') {
            Some((key, value)) => other.set(layer, key, value)?,
            None => return Err(format!("{pair:?} is not <key>=<value>")),
        }
    }
    Ok(other)
}

fn parse_kind(text: &str) -> Result<Kind, String> {
    match text {
        "text" => Ok(Kind::Text),
        "bool" => Ok(Kind::Bool),
        "integer" => Ok(Kind::Integer {
            min: i64::MIN,
            max: i64::MAX,
        }),
        _ => {
            if let Some(rest) = text.strip_prefix("integer:") {
                match rest.split_once(':') {
                    Some((min, max)) => match (min.parse(), max.parse()) {
                        (Ok(min), Ok(max)) => Ok(Kind::Integer { min, max }),
                        _ => Err(format!("{text:?} is not integer:<min>:<max>")),
                    },
                    None => Err(format!("{text:?} is not integer:<min>:<max>")),
                }
            } else if let Some(rest) = text.strip_prefix("enum:") {
                Ok(Kind::Enum(rest.split(',').map(str::to_owned).collect()))
            } else {
                Err(format!("{text:?} is not a setting kind"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_kind;
    use crate::app::settings::Kind;

    #[test]
    fn a_bounded_integer_and_an_enum_are_typed() {
        assert_eq!(
            parse_kind("integer:8:32").expect("a kind"),
            Kind::Integer { min: 8, max: 32 }
        );
        assert!(matches!(
            parse_kind("enum:light,dark").expect("a kind"),
            Kind::Enum(_)
        ));
        assert!(parse_kind("float").is_err());
    }
}
