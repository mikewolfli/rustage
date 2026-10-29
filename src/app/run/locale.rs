// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! F24: the interface language, its vocabulary report and the bundled language packs (task T24.03).
//!
//! This handler is where the identified vocabulary becomes observable: it prints which interface
//! language the process resolved to, what the vocabulary covers, and writes every bundled pack that
//! `--ui-language` and the designer's own diagnostics read — English and Simplified Chinese today. A
//! pack that does not read back as the one that was written is a failure, not a success with a note.

use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::app::locale::catalog;
use crate::app::locale::vocabulary;
use crate::host::locale::Language;

/// The moment the format line shows: 2023-11-14 22:13:20 UTC.
const SAMPLE_TIMESTAMP: i64 = 1_700_000_000;

/// The number the format line groups.
const SAMPLE_NUMBER: i64 = 1_234_567;

/// The names the format line orders, chosen so the two rules give different answers.
const SAMPLE_NAMES: [&str; 3] = ["Banana", "apple", "页面"];

/// One line from the interface catalog, with nothing to fill in.
pub(super) fn line(key: &str) -> String {
    line_with(key, &[])
}

/// One line from the interface catalog, with its `{name}` placeholders filled in.
pub(super) fn line_with(key: &str, arguments: &[(&str, &str)]) -> String {
    catalog::Catalog::current().fill(key, arguments)
}

/// The version banner, built from the catalog rather than from a literal.
pub(super) fn version_line() -> String {
    line_with("shell.version", &[("version", env!("CARGO_PKG_VERSION"))])
}

/// Writes the interface-vocabulary report and every bundled language pack (task T24.03).
pub(super) fn locale_report(directory: &Path) -> bool {
    // Which language the process is using, and why — reports the same selection the rest of the
    // process reads, so a report cannot name a language the process is not speaking.
    let selection = catalog::installed();
    println!("{}", selection.render());

    let coverage = vocabulary::coverage();
    println!("{}", coverage.render());
    if !coverage.is_complete() {
        for key in &coverage.uncovered {
            println!("uncovered: {key}");
        }
        return report("the interface vocabulary has a label with no key".to_owned());
    }

    let mut by_source: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in vocabulary::entries() {
        *by_source.entry(entry.source.name()).or_default() += 1;
    }
    for (source, count) in by_source {
        println!("{source}: {count}");
    }

    if let Err(error) = std::fs::create_dir_all(directory) {
        eprintln!("rustage: could not create {}: {error}", directory.display());
        return false;
    }

    // Every bundled language is written, not only the one this process happens to speak: the report
    // is the build's resource inventory, and a pack that exists but was never written would be a
    // language the build claims and no file carries.
    for language in Language::ALL {
        if !language.bundled() {
            continue;
        }
        let catalog = match catalog::Catalog::for_language(language) {
            Ok(catalog) => catalog,
            Err(refusal) => return report(refusal.message()),
        };
        println!(
            "bundled catalog: {} ({}) with {} message(s)",
            catalog.language().name(),
            catalog.language().tag(),
            catalog.pack().len()
        );

        // The language's dates, numbers and list order, shown as the interface would write them.
        // The ordering line names the rule, so a list is never described as collated when it is not.
        let rules = crate::app::locale::format::Rules::for_language(catalog.language());
        let sample: Vec<String> = SAMPLE_NAMES.iter().map(|name| (*name).to_owned()).collect();
        println!(
            "format: date {} | number {} | order {} — {} (sample: {})",
            rules.date_time(SAMPLE_TIMESTAMP),
            rules.number(SAMPLE_NUMBER),
            rules.collation().name(),
            rules.collation().note(),
            rules.sorted(&sample).join(" < ")
        );

        let path = directory.join(format!("{}.lang", catalog.language().tag()));
        if let Err(error) = std::fs::write(&path, catalog.pack().encode()) {
            eprintln!("rustage: could not write {}: {error}", path.display());
            return false;
        }

        // Read the file back, so what the report claims to have written is what is on disk.
        let read_back = std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| catalog::Pack::decode(&text).map_err(|fault| fault.message()));
        match read_back {
            Ok(pack) if pack == *catalog.pack() => {
                println!("{}", path.display());
            }
            Ok(_) => {
                return report("the written language pack did not read back the same".to_owned());
            }
            Err(reason) => return report(reason),
        }
    }
    true
}
