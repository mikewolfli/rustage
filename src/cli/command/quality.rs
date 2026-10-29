// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The problem centre, design checks, test evidence and visual conditions (task F21).

#![allow(clippy::needless_return)]

use super::*;

/// The quality evidence operations (task T21.01–T21.11).
#[derive(Debug, PartialEq, Eq)]
pub struct QualityOptions {
    /// The operations: `source:<name>`, `target:<name>`, `revision:<n>`, `node:<id>`, `location:<text>`,
    /// `severity:<error|warning|note>`, `problem:<message>`, `filter:<source>`, `filter-target:<name>`,
    /// `filter-revision:<n>`, `duplicate:<id>`, `parent:<id>:<parent>`, `capability:<name>:<true|false>`,
    /// `capacity:<used>:<limit>`, `resource:<name>:<true|false>`, `focus:<id>`, `check`,
    /// `contrast:<fg>:<bg>:<min>`, `test:<name>:<status>[:<seconds>]`, `tests`, `test-tree[:<filter>]`,
    /// `junit:<file>`,
    /// `junit-import:<file>`, `a11y-profile:<profile>`,
    /// `a11y-node:<id>:<role>:<focusable>:<focused>:<label>`, `a11y-child:<parent>:<child>`,
    /// `a11y-label-for:<node>:<label>`, `a11y-control-for:<label>:<node>`,
    /// `a11y-manual:<tool>:<result>:<note>`, `a11y`, `a11y-accept`,
    /// `visual:<platform>:<scale>:<font>[:<resources>]`,
    /// `visual-compare:<platform>:<scale>:<font>[:<resources>]`,
    /// `visual-tolerance:<percent>`, `visual-baseline:<name>:<w>:<h>:<digits>`,
    /// `visual-candidate:<name>:<w>:<h>:<digits>`, `visual-check`, `evidence:<file>`,
    /// `load[:<cycles>]`, `state`, `show`.
    ///
    /// The test wizard (T21.05): `wizard:<name>`, `w-state:<name>`, `w-initial:<state>`,
    /// `w-event:<name>:<kind>:<value>`, `w-transition:<from>:<event>:<to>`,
    /// `w-transition-if:<from>:<event>:<to>:<field>:<kind>:<value>`, `w-page:<name>`,
    /// `w-entry:<page>`, `w-edge:<from>:<to>`, `w-seam:<function>:<kind>:<value>`,
    /// `wizard-vectors`, `wizard-run`, `wizard-save:<file>`, `wizard-load:<file>`.
    pub ops: Vec<String>,
}

impl QualityOptions {
    /// Options with no facts.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

impl Default for QualityOptions {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn parse_quality(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--quality" => {
            let mut options = QualityOptions::new();
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok(Command::Quality(options)));
        }
        _ => None,
    }
}
