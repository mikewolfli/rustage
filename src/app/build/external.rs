// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! External tools: what they are, how their arguments are filled, and when a person is asked
//! (task T16.07).
//!
//! The acceptance line is *"a tool's arguments come from named placeholders, its input and output
//! locations are explicit, and a tool that could change something outside the project is confirmed
//! first"*.
//!
//! * **Placeholders are named, not positional.** [`expand`] replaces `${name}` with the caller's
//!   value and **refuses** an unknown placeholder rather than passing `${typo}` to a program as a
//!   literal — a tool silently run with a wrong argument is worse than one that did not run.
//! * **The input and output locations are fields.** [`Tool::inputs`] and [`Tool::outputs`] are where
//!   the designer watches, so "which file did this refresh?" is answerable without reading the argv.
//! * **Confirmation is required when the tool may touch outside the project.** [`Tool::confirm`] is
//!   set for a tool the file marks as external, and [`invoke`] refuses to run one unconfirmed.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// One tool the file defines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Tool {
    /// The tool's name.
    pub name: String,
    /// The program to run.
    pub program: String,
    /// The arguments, which may hold `${placeholder}`s.
    pub args: Vec<String>,
    /// The working directory, when it is not the project's.
    pub directory: Option<String>,
    /// The files the tool reads, for the input location.
    pub inputs: Vec<String>,
    /// The files the tool writes, for the output location.
    pub outputs: Vec<String>,
    /// Whether a person must confirm before it runs.
    pub confirm: bool,
}

/// One tool invocation, ready to run.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Invocation {
    /// The tool's name.
    pub tool: String,
    /// The argv, program first.
    pub argv: Vec<String>,
    /// The working directory.
    pub directory: Option<String>,
}

impl Invocation {
    /// The invocation in one line.
    pub fn render(&self) -> String {
        format!("{}: {}", self.tool, self.argv.join(" "))
    }
}

/// Expands a tool's arguments with the given placeholders.
///
/// A placeholder with no value is a refusal: a program run with `${file}` literally would act on a
/// path that does not exist, and reporting it is the honest outcome.
pub fn expand(
    args: &[String],
    placeholders: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        out.push(expand_one(arg, placeholders)?);
    }
    Ok(out)
}

/// Expands one argument.
fn expand_one(arg: &str, placeholders: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = arg;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(format!("{arg:?} has a `${{` with no closing `}}`"));
        };
        let name = &after[..end];
        match placeholders.get(name) {
            Some(value) => out.push_str(value),
            None => {
                return Err(format!(
                    "{arg:?} names the placeholder `${{{name}}}`, which has no value"
                ));
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Prepares a tool's invocation, or refuses when it is unconfirmed and external.
pub fn invoke(
    tool: &Tool,
    placeholders: &BTreeMap<String, String>,
    confirmed: bool,
) -> Result<Invocation, String> {
    if tool.confirm && !confirmed {
        return Err(format!(
            "tool {:?} may touch something outside the project and has not been confirmed",
            tool.name
        ));
    }
    let mut argv = vec![tool.program.clone()];
    argv.extend(expand(&tool.args, placeholders)?);
    Ok(Invocation {
        tool: tool.name.clone(),
        argv,
        directory: tool.directory.clone(),
    })
}

/// The placeholders a standard invocation supplies, with the values a caller gives.
pub fn standard_placeholders(
    project: &str,
    file: Option<&str>,
    node: Option<&str>,
    output: Option<&str>,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    map.insert("project".to_owned(), project.to_owned());
    if let Some(file) = file {
        map.insert("file".to_owned(), file.to_owned());
    }
    if let Some(node) = node {
        map.insert("node".to_owned(), node.to_owned());
    }
    if let Some(output) = output {
        map.insert("output".to_owned(), output.to_owned());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::{Tool, expand, invoke, standard_placeholders};

    fn tool(confirm: bool) -> Tool {
        Tool {
            name: "open".to_owned(),
            program: "editor".to_owned(),
            args: vec!["${file}".to_owned(), "--line".to_owned(), "12".to_owned()],
            directory: None,
            inputs: vec!["${file}".to_owned()],
            outputs: Vec::new(),
            confirm,
        }
    }

    #[test]
    fn a_placeholder_is_filled_from_its_value() {
        let placeholders = standard_placeholders("demo", Some("src/main.rs"), None, None);
        assert_eq!(
            expand(&tool(false).args, &placeholders).unwrap(),
            vec!["src/main.rs", "--line", "12"],
        );
    }

    #[test]
    fn an_unknown_placeholder_is_refused_rather_than_passed_literally() {
        let placeholders = standard_placeholders("demo", None, None, None);
        let error = expand(&tool(false).args, &placeholders).expect_err("a missing value");
        assert!(error.contains("file"), "{error}");
        assert!(error.contains("no value"), "{error}");
    }

    #[test]
    fn an_unclosed_placeholder_is_refused() {
        let error = expand(&["${oops".to_owned()], &Default::default()).expect_err("unclosed");
        assert!(error.contains("no closing"), "{error}");
    }

    #[test]
    fn an_external_tool_is_confirmed_before_it_runs() {
        let placeholders = standard_placeholders("demo", Some("a.rs"), None, None);
        let error = invoke(&tool(true), &placeholders, false).expect_err("unconfirmed");
        assert!(error.contains("not been confirmed"), "{error}");
        let invocation = invoke(&tool(true), &placeholders, true).expect("a confirmed invocation");
        assert_eq!(invocation.argv[0], "editor");
        assert!(invocation.render().contains("a.rs"));
    }

    #[test]
    fn an_input_location_names_the_file_the_designer_watches() {
        let placeholders = standard_placeholders("demo", Some("src/main.rs"), None, None);
        let filled = expand(&tool(false).inputs, &placeholders).unwrap();
        assert_eq!(filled, vec!["src/main.rs"]);
    }
}
