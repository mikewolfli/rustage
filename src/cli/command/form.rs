// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The form family's command: typed validation rules, submit, reset and async staleness
//! (task T13.09).

use super::*;

/// How to build a form, validate a draft and decide whether it may be submitted (task T13.09).
#[derive(Debug, PartialEq, Eq)]
pub struct FormOptions {
    /// The operations: `field:<name>:<kind>`, `required:<name>`, `same-as:<name>:<other>`,
    /// `different-from:<name>:<other>`, `required-when:<name>:<other>=<value>`,
    /// `min-length:<name>:<n>`, `at-least:<name>:<n>`, `one-of:<name>:<a>|<b>`,
    /// `value:<name>=<text>`, `revision:<n>`, `pending:<field>:<revision>`,
    /// `mode:<preview|runtime|static>`, `validate`, `submit`, `reset`, `show`.
    pub ops: Vec<String>,
}

impl FormOptions {
    /// Options with no form.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_form(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    macro_rules! ops {
        ($options:expr, $variant:path) => {{
            let mut options = $options;
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok($variant(options)));
        }};
    }
    match text {
        "--form" => ops!(FormOptions::new(), Command::Form),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_form_operations() {
        assert_eq!(parse(["--form"]), Ok(Command::Form(FormOptions::new())));
        let mut expected = FormOptions::new();
        expected.ops = vec!["field:name:text".to_owned(), "submit".to_owned()];
        assert_eq!(
            parse(["--form", "field:name:text", "submit"]),
            Ok(Command::Form(expected)),
        );
        assert!(parse(["--form", "--oops"]).is_err());
    }
}
