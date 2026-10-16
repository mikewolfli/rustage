// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! What is written where, and which bytes a person owns (task T15.01).
//!
//! The acceptance line is *"the output granularity and the commit decision are reproducible"*. Both
//! halves are made into shapes rather than conventions:
//!
//! * **One design source has one generated file.** [`plan`] turns the project document's
//!   `generated` entries into a [`Plan`], refusing two outputs for one source and two sources for
//!   one output. A project whose plan is not a function is refused here rather than at write time,
//!   where a second run would pick a different file.
//! * **The plan is ordered, not "whatever the reader saw".** Entries are sorted by source path, so
//!   the same project document produces the same plan regardless of member order (principle 16).
//! * **The commit decision is a value.** Decision `D7-b-3` in the library is *generated sources are
//!   committed*; [`CommitPolicy::Committed`] states that in the plan, so "is this file ours to
//!   overwrite?" is answered by the plan and not by a heuristic at the write site.
//!
//! ## Generated regions and hand-written boundaries
//!
//! A generated file is not wholly the generator's: a person may add code between explicit
//! boundaries, and regeneration must leave those bytes alone (principle 19). The boundaries are
//! ordinary line comments, so a file that lost its tooling is still readable:
//!
//! ```text
//! // region:handwritten <name>
//! ... kept verbatim across regeneration ...
//! // endregion:handwritten <name>
//! ```
//!
//! [`regions`] walks a file into [`Region`]s and refuses the two shapes a merge cannot honour: an
//! end marker with no start, and a start that is never closed. A file that does not open with the
//! library's [`FILE_MARKER`] is not a generated file at all, and [`is_generated`] says so instead of
//! letting the merge treat a hand-written file as safe to overwrite.

#![allow(dead_code)]

use std::collections::BTreeMap;

/// The first line of every file the library's generator writes.
///
/// One definition, imported from the generator that writes it: a second copy here would be a second
/// answer to "is this file generated", and the two would drift exactly when it mattered.
pub const FILE_MARKER: &str = rust_widgets::designer::GENERATED_MARKER;

/// The prefix of a hand-written region's opening marker, before the region name.
pub const HANDWRITTEN_OPEN: &str = "// region:handwritten";

/// The prefix of a hand-written region's closing marker, before the region name.
pub const HANDWRITTEN_CLOSE: &str = "// endregion:handwritten";

/// The prefix of a generated region's opening marker, before the region name.
pub const GENERATED_OPEN: &str = "// region:generated";

/// The prefix of a generated region's closing marker, before the region name.
pub const GENERATED_CLOSE: &str = "// endregion:generated";

/// Which side of the boundary a span of a generated file is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionKind {
    /// Bytes the generator owns; a regeneration replaces them.
    Generated,
    /// Bytes a person owns; a regeneration keeps them.
    Handwritten,
}

impl RegionKind {
    /// The kind's name, as its marker spells it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Handwritten => "handwritten",
        }
    }

    /// The marker that opens a region of this kind.
    pub fn opener(&self) -> &'static str {
        match self {
            Self::Generated => GENERATED_OPEN,
            Self::Handwritten => HANDWRITTEN_OPEN,
        }
    }

    /// The marker that closes a region of this kind.
    pub fn closer(&self) -> &'static str {
        match self {
            Self::Generated => GENERATED_CLOSE,
            Self::Handwritten => HANDWRITTEN_CLOSE,
        }
    }
}

/// One span of a generated file, and which side of the boundary owns it.
///
/// `start` and `end` bound the region's **content**, not its markers: a merge copies
/// `text[start..end]` and re-emits the markers around it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Region {
    /// Who owns these bytes.
    pub kind: RegionKind,
    /// The region's name, empty for the file's own default body.
    pub name: String,
    /// The first byte of the content.
    pub start: u32,
    /// One past the last byte of the content.
    pub end: u32,
}

impl Region {
    /// The region's content, as a slice of the file it was read from.
    pub fn content<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start as usize..self.end as usize]
    }

    /// The region in one line.
    pub fn render(&self) -> String {
        if self.name.is_empty() {
            return format!("{} {}..{}", self.kind.name(), self.start, self.end);
        }
        format!(
            "{} {:<12} {}..{}",
            self.kind.name(),
            self.name,
            self.start,
            self.end
        )
    }
}

/// Whether `text` is a file this crate generated.
///
/// The first non-empty line decides, so a file that merely quotes the marker is not mistaken for a
/// generated one — the same rule the library's own gate uses.
pub fn is_generated(text: &str) -> bool {
    rust_widgets::designer::is_generated(text)
}

/// Splits a generated file into the regions a merge has to respect.
///
/// The walk is line-based because the markers are lines; offsets are byte offsets into `text`, so a
/// Unicode file's regions still slice correctly. A stray close, or an opener that is never closed,
/// is refused: a merge that guessed which region a marker belonged to would delete the wrong bytes.
pub fn regions(text: &str) -> Result<Vec<Region>, String> {
    let mut out: Vec<Region> = Vec::new();
    // The opener still looking for its close: its kind, its name, and where its content begins.
    let mut open: Option<(RegionKind, String, u32)> = None;
    let mut body_start: u32 = 0;
    let mut offset: u32 = 0;

    for line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len() as u32;
        let trimmed = line.trim_end_matches(['\n', '\r']).trim();

        if let Some((kind, name, content_start)) = &open
            && trimmed == closer_line(*kind, name)
        {
            let start = *content_start;
            let end = line_start;
            // A named region is kept even when its body is empty: an empty placeholder is how a
            // regenerated body says "a hand-written region belongs here", and dropping it would
            // make the merge unable to place the region it stands for.
            if start < end || !name.is_empty() {
                out.push(Region {
                    kind: *kind,
                    name: name.clone(),
                    start,
                    end,
                });
            }
            open = None;
            body_start = offset;
            continue;
        }

        if open.is_none() {
            if let Some((kind, name)) = open_marker(trimmed) {
                if body_start < line_start {
                    out.push(Region {
                        kind: RegionKind::Generated,
                        name: String::new(),
                        start: body_start,
                        end: line_start,
                    });
                }
                open = Some((kind, name, offset));
                continue;
            }
            if trimmed.starts_with(HANDWRITTEN_CLOSE) || trimmed.starts_with(GENERATED_CLOSE) {
                return Err(format!("`{trimmed}` closes a region that was not open"));
            }
        }
    }

    if let Some((kind, name, _)) = open {
        return Err(format!("{} region {name:?} is never closed", kind.name()));
    }
    if body_start < text.len() as u32 {
        out.push(Region {
            kind: RegionKind::Generated,
            name: String::new(),
            start: body_start,
            end: text.len() as u32,
        });
    }
    Ok(out)
}

/// The exact line that closes `kind`'s region named `name`.
fn closer_line(kind: RegionKind, name: &str) -> String {
    if name.is_empty() {
        kind.closer().to_owned()
    } else {
        format!("{} {name}", kind.closer())
    }
}

/// The region an opening marker line declares, or `None` when the line is not one.
///
/// A bare opener with no name is a region named the empty string, so this answers `Some` for it
/// rather than treating it as "not a marker".
fn open_marker(line: &str) -> Option<(RegionKind, String)> {
    for kind in [RegionKind::Generated, RegionKind::Handwritten] {
        if let Some(rest) = line.strip_prefix(kind.opener()) {
            if rest.is_empty() {
                return Some((kind, String::new()));
            }
            if let Some(name) = rest.strip_prefix(' ') {
                return Some((kind, name.trim().to_owned()));
            }
        }
    }
    None
}

/// How a generated file's content is kept in version control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommitPolicy {
    /// The generated file is written to the project tree and committed alongside its source, so a
    /// change to it is a reviewable diff (decision `D7-b-3`).
    Committed,
}

impl CommitPolicy {
    /// The policy in one line.
    pub fn render(&self) -> &'static str {
        match self {
            Self::Committed => "committed to the project tree (reviewable as a diff)",
        }
    }
}

/// One design source and the file it generates.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Planned {
    /// The page document this comes from, project-relative.
    pub source: String,
    /// The file the generator writes, project-relative.
    pub output: String,
}

/// Every file a project generates, and how the result is kept.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// How the generated files are kept.
    pub policy: CommitPolicy,
    /// The sources and their outputs, sorted by source.
    pub artifacts: Vec<Planned>,
}

impl Plan {
    /// The plan in one line per artifact.
    pub fn render(&self) -> Vec<String> {
        let mut out = vec![format!("policy: {}", self.policy.render())];
        out.extend(
            self.artifacts
                .iter()
                .map(|artifact| format!("{} -> {}", artifact.source, artifact.output)),
        );
        out
    }

    /// The output for a source, if the plan writes one.
    pub fn output_for(&self, source: &str) -> Option<&str> {
        self.artifacts
            .iter()
            .find(|artifact| artifact.source == source)
            .map(|artifact| artifact.output.as_str())
    }
}

/// Turns a project document's `generated` entries into a plan, or explains why it cannot.
///
/// Each entry is `(source, output)` — the page document and the file generated from it. Entries are
/// sorted by source so the plan is a function of the document, and three shapes are refused because
/// no write site could honour them:
///
/// * a blank source or output (there is nowhere to read or write);
/// * two outputs for one source (which file is the generated one?);
/// * two sources for one output (a later write would clobber an earlier one).
pub fn plan(targets: &[(String, String)]) -> Result<Plan, String> {
    let mut by_source: BTreeMap<String, String> = BTreeMap::new();
    for (source, output) in targets {
        let source = source.trim();
        let output = output.trim();
        if source.is_empty() || output.is_empty() {
            return Err(format!(
                "a generated entry names a blank {}",
                if source.is_empty() {
                    "source"
                } else {
                    "output"
                }
            ));
        }
        if let Some(existing) = by_source.get(source) {
            return Err(format!(
                "{source} would generate two files ({existing} and {output}); a source generates \
                 exactly one"
            ));
        }
        if let Some((other, _)) = by_source.iter().find(|(_, value)| value.as_str() == output) {
            return Err(format!(
                "{output} would be generated by two sources ({other} and {source})"
            ));
        }
        by_source.insert(source.to_owned(), output.to_owned());
    }
    let artifacts = by_source
        .into_iter()
        .map(|(source, output)| Planned { source, output })
        .collect();
    Ok(Plan {
        policy: CommitPolicy::Committed,
        artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CommitPolicy, FILE_MARKER, Plan, Planned, Region, RegionKind, is_generated, plan, regions,
    };

    const GENERATED: &str = "// Generated by the rust_widgets designer. Do not edit by hand.\n\
         pub fn build_ui() {\n\
         }\n";

    #[test]
    fn the_file_marker_is_the_one_the_generator_writes() {
        assert!(GENERATED.trim_start().starts_with(FILE_MARKER));
    }

    #[test]
    fn a_generated_file_is_recognised_by_its_first_non_empty_line() {
        assert!(is_generated(GENERATED));
        assert!(is_generated(&format!("\n\n{GENERATED}")));
        assert!(!is_generated("pub fn main() {}\n"));
        // A file that merely quotes the marker is not a generated file.
        assert!(!is_generated(&format!("// see {FILE_MARKER}\n")));
    }

    #[test]
    fn a_file_with_no_regions_is_one_generated_region() {
        let parsed = regions(GENERATED).expect("regions");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].kind, RegionKind::Generated);
        assert_eq!(parsed[0].content(GENERATED), GENERATED);
    }

    #[test]
    fn a_handwritten_region_is_kept_between_its_markers() {
        let text = format!(
            "{GENERATED}// region:handwritten save\nfn save() {{ /* mine */ }}\n// endregion:handwritten save\n"
        );
        let parsed = regions(&text).expect("regions");

        let hand: Vec<&Region> = parsed
            .iter()
            .filter(|region| region.kind == RegionKind::Handwritten)
            .collect();
        assert_eq!(hand.len(), 1);
        assert_eq!(hand[0].name, "save");
        assert!(hand[0].content(&text).contains("fn save()"));
        assert!(!hand[0].content(&text).contains("endregion"));
    }

    #[test]
    fn the_regions_of_a_mixed_file_tile_it_without_gaps_or_overlap() {
        let text = format!(
            "{GENERATED}// region:handwritten a\nlet a = 1;\n// endregion:handwritten a\n\
             fn generated_between() {{}}\n\
             // region:handwritten b\nlet b = 2;\n// endregion:handwritten b\n"
        );
        let parsed = regions(&text).expect("regions");

        let total: u32 = parsed.iter().map(|region| region.end - region.start).sum();
        assert!(total <= text.len() as u32);
        // Every region slices valid UTF-8.
        for region in &parsed {
            let _ = region.content(&text);
        }
        assert_eq!(
            parsed
                .iter()
                .filter(|r| r.kind == RegionKind::Handwritten)
                .count(),
            2,
        );
    }

    #[test]
    fn an_unclosed_region_is_refused() {
        let text = format!("{GENERATED}// region:handwritten keep\nfn keep() {{}}\n");
        let error = regions(&text).expect_err("an unclosed region");
        assert!(error.contains("never closed"), "{error}");
    }

    #[test]
    fn a_close_without_an_open_is_refused() {
        let text = format!("{GENERATED}// endregion:handwritten save\n");
        let error = regions(&text).expect_err("a stray close");
        assert!(error.contains("not open"), "{error}");
    }

    #[test]
    fn a_plan_orders_by_source_and_refuses_a_source_with_two_outputs() {
        let built = plan(&[
            ("pages/b.json".to_owned(), "src/generated/b.rs".to_owned()),
            ("pages/a.json".to_owned(), "src/generated/a.rs".to_owned()),
        ])
        .expect("a plan");
        assert_eq!(built.policy, CommitPolicy::Committed);
        assert_eq!(built.output_for("pages/a.json"), Some("src/generated/a.rs"));
        assert_eq!(
            built
                .artifacts
                .iter()
                .map(|a| a.source.as_str())
                .collect::<Vec<_>>(),
            vec!["pages/a.json", "pages/b.json"],
        );

        let error = plan(&[
            ("pages/a.json".to_owned(), "src/a.rs".to_owned()),
            ("pages/a.json".to_owned(), "src/b.rs".to_owned()),
        ])
        .expect_err("two outputs");
        assert!(error.contains("two files"), "{error}");
    }

    #[test]
    fn a_plan_refuses_two_sources_for_one_output_and_blank_paths() {
        let error = plan(&[
            ("pages/a.json".to_owned(), "src/generated/ui.rs".to_owned()),
            ("pages/b.json".to_owned(), "src/generated/ui.rs".to_owned()),
        ])
        .expect_err("one output, two sources");
        assert!(error.contains("two sources"), "{error}");

        assert!(plan(&[("  ".to_owned(), "src/ui.rs".to_owned())]).is_err());
        assert!(plan(&[("pages/a.json".to_owned(), "".to_owned())]).is_err());
    }

    #[test]
    fn a_plan_renders_the_commit_decision_then_the_artifacts() {
        let plan = Plan {
            policy: CommitPolicy::Committed,
            artifacts: vec![Planned {
                source: "pages/main.json".to_owned(),
                output: "src/generated/main.rs".to_owned(),
            }],
        };
        let rendered = plan.render();
        assert!(rendered[0].starts_with("policy: committed"));
        assert_eq!(rendered[1], "pages/main.json -> src/generated/main.rs");
    }
}
