// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The five-profile certification report and the IPC/static-core consistency evidence
//! (task T18.08).
//!
//! The acceptance line is *"generation, check, build, run and device are separate columns, and the
//! report has unverified items"*. The report is honest by construction:
//!
//! * **A column is a stage, not a summary.** [`Stage`] has five variants and each [`Cell`] belongs
//!   to exactly one, so a build cannot stand in for a run and a generator cannot stand in for a
//!   device.
//! * **A cell needs evidence, not a level.** [`Cell::new`] refuses an empty command or an empty
//!   observation, because a level typed into a report with nothing behind it is exactly the false
//!   success this task exists to prevent.
//! * **Unverified is a displayed state.** [`Report::new`] starts with every cell unverified, and
//!   [`Report::unverified`] lists each one by profile and stage. [`Report::is_certified`] is false
//!   until every cell has evidence and the consistency measurement agrees.
//! * **The IPC/static agreement is measured, not asserted.** [`measure_consistency`] drives the
//!   real shared core directly and again through the typed channel (F17's own regression harness),
//!   so the "static and IPC cores agree" line in the report is a result rather than a claim.

#![allow(dead_code)]

use crate::app::logic::core::{Core, Event, Seam, Transition, Value};
use crate::app::logic::regression::{self, Vector};

use super::matrix::Proof;

/// The five profiles the report covers, in roadmap order.
pub const PROFILES: [&str; 5] = ["desktop", "tablet", "mobile", "mini", "embedded"];

/// One column of the certification report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Source was generated for the profile.
    Generate,
    /// Cargo checked it.
    Check,
    /// Cargo built it.
    Build,
    /// It ran.
    Run,
    /// It ran on the device the profile names.
    Device,
}

impl Stage {
    /// The stage's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::Check => "check",
            Self::Build => "build",
            Self::Run => "run",
            Self::Device => "device",
        }
    }

    /// Reads a stage name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "generate" => Some(Self::Generate),
            "check" => Some(Self::Check),
            "build" => Some(Self::Build),
            "run" => Some(Self::Run),
            "device" => Some(Self::Device),
            _ => None,
        }
    }

    /// Every stage, in report order.
    pub fn all() -> [Self; 5] {
        [
            Self::Generate,
            Self::Check,
            Self::Build,
            Self::Run,
            Self::Device,
        ]
    }

    /// The stage's column.
    pub fn index(&self) -> usize {
        match self {
            Self::Generate => 0,
            Self::Check => 1,
            Self::Build => 2,
            Self::Run => 3,
            Self::Device => 4,
        }
    }
}

/// One cell: what a stage reached, and the evidence for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    proof: Option<Proof>,
    command: String,
    note: String,
}

impl Cell {
    /// A cell with no evidence.
    pub fn unverified() -> Self {
        Self {
            proof: None,
            command: String::new(),
            note: String::new(),
        }
    }

    /// A cell backed by a command and an observation.
    ///
    /// Both are required: "we built it" with no command and no observation is exactly the claim
    /// this report must not accept.
    pub fn new(
        proof: Proof,
        command: impl Into<String>,
        note: impl Into<String>,
    ) -> Result<Self, String> {
        let command = command.into();
        let note = note.into();
        if command.trim().is_empty() {
            return Err("a certified stage needs the command that produced it".to_owned());
        }
        if note.trim().is_empty() {
            return Err("a certified stage needs what the command showed".to_owned());
        }
        Ok(Self {
            proof: Some(proof),
            command,
            note,
        })
    }

    /// What the stage reached, or `None` when it was not verified.
    pub fn proof(&self) -> Option<Proof> {
        self.proof
    }

    /// Whether this cell has evidence.
    pub fn is_verified(&self) -> bool {
        self.proof.is_some()
    }

    /// The cell in one line.
    pub fn render(&self) -> String {
        match &self.proof {
            None => "unverified".to_owned(),
            Some(proof) => {
                let note = if self.note.is_empty() {
                    String::new()
                } else {
                    format!(": {}", self.note)
                };
                format!("{} ({} {note})", proof.name(), self.command)
                    .trim_end()
                    .to_owned()
            }
        }
    }
}

/// One profile's row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    /// The profile.
    pub profile: String,
    /// One cell per stage, in [`Stage::all`] order.
    pub cells: [Cell; 5],
}

impl Row {
    /// A row with every cell unverified.
    pub fn new(profile: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            cells: std::array::from_fn(|_| Cell::unverified()),
        }
    }

    /// The row in one line.
    pub fn render(&self) -> String {
        let cells: Vec<String> = self.cells.iter().map(Cell::render).collect();
        format!("{} | {}", self.profile, cells.join(" | "))
    }
}

/// The whole certification report.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    rows: Vec<Row>,
    consistency: Vec<regression::Report>,
}

impl Report {
    /// Five rows, every cell unverified.
    pub fn new() -> Self {
        Self {
            rows: PROFILES.iter().map(|profile| Row::new(*profile)).collect(),
            consistency: Vec::new(),
        }
    }

    /// Every row, in roadmap order.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Records evidence for one profile's stage.
    pub fn record(
        &mut self,
        profile: &str,
        stage: Stage,
        proof: Proof,
        command: impl Into<String>,
        note: impl Into<String>,
    ) -> Result<(), String> {
        let cell = Cell::new(proof, command, note)?;
        let Some(row) = self.rows.iter_mut().find(|row| row.profile == profile) else {
            return Err(format!(
                "{profile:?} is not one of the five profiles ({})",
                PROFILES.join(", ")
            ));
        };
        row.cells[stage.index()] = cell;
        Ok(())
    }

    /// Adds the consistency measurement's reports.
    pub fn add_consistency(&mut self, reports: Vec<regression::Report>) {
        self.consistency = reports;
    }

    /// Whether the static and IPC seams agreed.
    ///
    /// `None` when no consistency measurement was recorded — which is not the same as `Some(true)`.
    pub fn consistency_holds(&self) -> Option<bool> {
        if self.consistency.is_empty() {
            None
        } else {
            Some(self.consistency.iter().all(|report| report.identical))
        }
    }

    /// Every cell without evidence, as `profile/stage`.
    pub fn unverified(&self) -> Vec<String> {
        let mut unverified = Vec::new();
        for row in &self.rows {
            for (stage, cell) in Stage::all().iter().zip(&row.cells) {
                if !cell.is_verified() {
                    unverified.push(format!("{}/{}", row.profile, stage.name()));
                }
            }
        }
        unverified
    }

    /// Whether every column has evidence and the two seams agree.
    pub fn is_certified(&self) -> bool {
        self.unverified().is_empty() && self.consistency_holds() == Some(true)
    }

    /// The report in one line per row, with the consistency line last.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "profile | {}",
            Stage::all()
                .iter()
                .map(Stage::name)
                .collect::<Vec<&str>>()
                .join(" | ")
        )];
        lines.extend(self.rows.iter().map(Row::render));
        lines.push(match self.consistency_holds() {
            None => "consistency: not measured".to_owned(),
            Some(true) => format!(
                "consistency: identical over {} vector(s)",
                self.consistency.len()
            ),
            Some(false) => "consistency: diverged".to_owned(),
        });
        lines
    }
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs one canonical event vector over both call seams.
///
/// The vector is small on purpose: this is a consistency check, not a scenario library. The point
/// is that the same transition is driven once by a compiled-in caller and once through the typed
/// channel, and the two state sequences are compared.
pub fn measure_consistency() -> Result<Vec<regression::Report>, String> {
    let mut core = Core::new("idle");
    core.add(Transition::new("idle", "go", "running"))
        .map_err(|refusal| refusal.render())?;
    let vector = Vector::new("certification").say(Event::new("go", Value::Number(1)));
    let seam = Seam::new();
    regression::run(&[vector], &core, &core.start(), &seam).map_err(|error| error.render())
}

#[cfg(test)]
mod tests {
    use super::{PROFILES, Report, Stage, measure_consistency};
    use crate::app::logic::regression;
    use crate::app::targets::matrix::Proof;

    #[test]
    fn a_new_report_has_five_rows_and_twenty_five_unverified_cells() {
        let report = Report::new();
        assert_eq!(report.rows().len(), 5);
        assert_eq!(report.unverified().len(), 25);
        assert!(!report.is_certified());
        assert_eq!(report.consistency_holds(), None);
    }

    #[test]
    fn every_stage_round_trips_through_its_name() {
        for stage in Stage::all() {
            assert_eq!(Stage::parse(stage.name()), Some(stage));
            assert_eq!(Stage::all()[stage.index()], stage);
        }
        assert!(Stage::parse("deploy").is_none());
    }

    #[test]
    fn evidence_needs_a_command_and_an_observation() {
        assert!(super::Cell::new(Proof::Built, "", "a binary").is_err());
        assert!(super::Cell::new(Proof::Built, "cargo build", "  ").is_err());
        assert!(super::Cell::new(Proof::Built, "cargo build", "ok").is_ok());
    }

    #[test]
    fn recording_evidence_clears_one_cell_and_names_the_rest() {
        let mut report = Report::new();
        report
            .record(
                "desktop",
                Stage::Build,
                Proof::Built,
                "cargo build",
                "target/debug/rustage",
            )
            .expect("evidence");
        let unverified = report.unverified();
        assert_eq!(unverified.len(), 24);
        assert!(!unverified.contains(&"desktop/build".to_owned()));
        assert!(unverified.contains(&"desktop/run".to_owned()));
        assert!(
            report
                .render()
                .iter()
                .any(|line| line.contains("desktop | unverified | unverified | built"))
        );
    }

    #[test]
    fn an_unknown_profile_is_refused() {
        let mut report = Report::new();
        assert!(
            report
                .record("watch", Stage::Build, Proof::Built, "cargo build", "ok")
                .is_err()
        );
        for profile in PROFILES {
            assert!(
                report
                    .record(profile, Stage::Generate, Proof::Generated, "generate", "ok")
                    .is_ok()
            );
        }
    }

    #[test]
    fn the_consistency_measurement_runs_over_both_seams() {
        let reports = measure_consistency().expect("a measurement");
        assert_eq!(reports.len(), 1);
        assert!(reports[0].identical);
        assert!(reports[0].render().contains("identical"));
    }

    #[test]
    fn a_full_report_with_an_agreeing_measurement_is_certified() {
        let mut report = Report::new();
        for profile in PROFILES {
            for stage in Stage::all() {
                let proof = match stage {
                    Stage::Generate => Proof::Generated,
                    Stage::Check => Proof::Checked,
                    Stage::Build => Proof::Built,
                    Stage::Run => Proof::Ran,
                    Stage::Device => Proof::Device,
                };
                report
                    .record(profile, stage, proof, "cargo", "observed")
                    .expect("evidence");
            }
        }
        report.add_consistency(measure_consistency().expect("a measurement"));
        assert!(report.unverified().is_empty());
        assert_eq!(report.consistency_holds(), Some(true));
        assert!(report.is_certified());
        assert!(
            report
                .render()
                .last()
                .is_some_and(|line| line.contains("identical"))
        );
    }

    #[test]
    fn a_diverged_measurement_keeps_the_report_uncertified() {
        let mut report = Report::new();
        for profile in PROFILES {
            for stage in Stage::all() {
                report
                    .record(profile, stage, Proof::Device, "cargo", "observed")
                    .expect("evidence");
            }
        }
        report.add_consistency(vec![regression::Report {
            name: "diverged".to_owned(),
            steps: 2,
            identical: false,
        }]);
        assert_eq!(report.consistency_holds(), Some(false));
        assert!(!report.is_certified());
        assert!(
            report
                .render()
                .last()
                .is_some_and(|line| line.contains("diverged"))
        );
    }
}
