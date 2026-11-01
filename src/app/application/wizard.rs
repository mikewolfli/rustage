// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The one wizard shell every required wizard runs in (roadmap §3.4).
//!
//! The acceptance lines are *"wizards support a previous step and cancel; the last step shows the
//! final change and what a failure would leave behind; a cancel cannot leave half a project"*.
//! The rules that make each true:
//!
//! * **The shell is shared, the content is not invented.** [`RequiredWizard::ALL`] and
//!   [`RequiredConfirmation::ALL`] are exactly the wizards and confirmations roadmap §3.4 names; a
//!   caller supplies the input steps, so the shell does not pretend to know what a given wizard asks.
//! * **A review is the last step, and it carries the whole change.** [`Wizard::add_review`] refuses
//!   to place anything after it, and [`Wizard::finish`] refuses until the review is current, so a
//!   wizard that is not finished cannot look finished.
//! * **Nothing is applied before finish.** Steps only describe what will happen; [`Wizard::finish`]
//!   yields a [`Commit`], and [`Wizard::cancel`] discards the steps instead. There is no half state to
//!   leave behind because the shell never writes one.
//! * **Back is bounded.** [`Wizard::back`] refuses on the first step and [`Wizard::next`] on the last,
//!   so the walk is finite and cannot run past the review.

/// A wizard roadmap §3.4 requires.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum RequiredWizard {
    /// Creating a project.
    NewProject,
    /// Adding a build or deployment target.
    AddTarget,
    /// Importing an existing project.
    ImportProject,
    /// Importing an asset.
    ImportAsset,
    /// Importing a component.
    ImportComponent,
    /// Extracting a component from a page.
    ExtractComponent,
    /// Migrating a file's format.
    FormatMigration,
    /// Replacing a project.
    ProjectReplace,
    /// Resolving a code round-trip conflict.
    CodeRoundtripConflict,
    /// Migrating business state.
    StateMigration,
    /// Deploying to a device.
    DeviceDeploy,
    /// Packaging a release.
    PackagePublish,
    /// Diagnosing the first toolchain.
    ToolchainDiagnose,
}

impl RequiredWizard {
    /// Every required wizard, in the roadmap's order.
    pub const ALL: [Self; 13] = [
        Self::NewProject,
        Self::AddTarget,
        Self::ImportProject,
        Self::ImportAsset,
        Self::ImportComponent,
        Self::ExtractComponent,
        Self::FormatMigration,
        Self::ProjectReplace,
        Self::CodeRoundtripConflict,
        Self::StateMigration,
        Self::DeviceDeploy,
        Self::PackagePublish,
        Self::ToolchainDiagnose,
    ];

    /// Its stable id.
    pub const fn id(self) -> &'static str {
        match self {
            Self::NewProject => "wizard.new-project",
            Self::AddTarget => "wizard.add-target",
            Self::ImportProject => "wizard.import-project",
            Self::ImportAsset => "wizard.import-asset",
            Self::ImportComponent => "wizard.import-component",
            Self::ExtractComponent => "wizard.extract-component",
            Self::FormatMigration => "wizard.format-migration",
            Self::ProjectReplace => "wizard.project-replace",
            Self::CodeRoundtripConflict => "wizard.code-roundtrip-conflict",
            Self::StateMigration => "wizard.state-migration",
            Self::DeviceDeploy => "wizard.device-deploy",
            Self::PackagePublish => "wizard.package-publish",
            Self::ToolchainDiagnose => "wizard.toolchain-diagnose",
        }
    }

    /// Its name in the roadmap's words.
    pub const fn title(self) -> &'static str {
        match self {
            Self::NewProject => "New project",
            Self::AddTarget => "Add target",
            Self::ImportProject => "Import project",
            Self::ImportAsset => "Import asset",
            Self::ImportComponent => "Import component",
            Self::ExtractComponent => "Extract component",
            Self::FormatMigration => "Format migration",
            Self::ProjectReplace => "Project replace",
            Self::CodeRoundtripConflict => "Code round-trip conflict",
            Self::StateMigration => "State migration",
            Self::DeviceDeploy => "Device deploy",
            Self::PackagePublish => "Package publish",
            Self::ToolchainDiagnose => "First toolchain diagnosis",
        }
    }

    /// The wizard an id names, when it names one.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|wizard| wizard.id() == id)
    }
}

/// A confirmation roadmap §3.4 requires before a destructive or external action.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum RequiredConfirmation {
    /// Closing a project with unsaved changes.
    CloseDirtyProject,
    /// Deleting an object something references.
    DeleteReferenced,
    /// Overwriting a file outside the project.
    OverwriteExternal,
    /// Resetting business state.
    ResetBusinessState,
    /// Running a command that is not trusted.
    RunUntrustedCommand,
    /// A destructive version-control operation.
    DestructiveVersionOperation,
    /// Sending context outside the machine.
    ShareContext,
    /// Updating or uninstalling an extension.
    UpdateAndUninstallExtension,
}

impl RequiredConfirmation {
    /// Every required confirmation, in the roadmap's order.
    pub const ALL: [Self; 8] = [
        Self::CloseDirtyProject,
        Self::DeleteReferenced,
        Self::OverwriteExternal,
        Self::ResetBusinessState,
        Self::RunUntrustedCommand,
        Self::DestructiveVersionOperation,
        Self::ShareContext,
        Self::UpdateAndUninstallExtension,
    ];

    /// Its stable id.
    pub const fn id(self) -> &'static str {
        match self {
            Self::CloseDirtyProject => "confirm.close-dirty-project",
            Self::DeleteReferenced => "confirm.delete-referenced",
            Self::OverwriteExternal => "confirm.overwrite-external",
            Self::ResetBusinessState => "confirm.reset-business-state",
            Self::RunUntrustedCommand => "confirm.run-untrusted-command",
            Self::DestructiveVersionOperation => "confirm.destructive-version-operation",
            Self::ShareContext => "confirm.share-context",
            Self::UpdateAndUninstallExtension => "confirm.update-and-uninstall-extension",
        }
    }

    /// Its name in the roadmap's words.
    pub const fn title(self) -> &'static str {
        match self {
            Self::CloseDirtyProject => "Close a project with unsaved changes",
            Self::DeleteReferenced => "Delete a referenced object",
            Self::OverwriteExternal => "Overwrite a file outside the project",
            Self::ResetBusinessState => "Reset business state",
            Self::RunUntrustedCommand => "Run an untrusted command",
            Self::DestructiveVersionOperation => "A destructive version operation",
            Self::ShareContext => "Send context outside the machine",
            Self::UpdateAndUninstallExtension => "Update or uninstall an extension",
        }
    }

    /// The confirmation an id names, when it names one.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|confirmation| confirmation.id() == id)
    }
}

/// One step of a wizard.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Step {
    /// A step that gathers input. It has no side effect.
    Input {
        /// Its stable id.
        id: String,
        /// What it asks.
        title: String,
    },
    /// The last step: the whole change, and what a failure would leave behind.
    Review {
        /// Every change the wizard would make.
        changes: Vec<String>,
        /// What a failure part-way would leave behind.
        failure_impact: String,
    },
}

impl Step {
    /// An input step.
    pub fn input(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::Input {
            id: id.into(),
            title: title.into(),
        }
    }

    /// A review step.
    pub fn review(changes: Vec<String>, failure_impact: impl Into<String>) -> Self {
        Self::Review {
            changes,
            failure_impact: failure_impact.into(),
        }
    }

    /// Its title.
    pub fn title(&self) -> &str {
        match self {
            Self::Input { title, .. } => title,
            Self::Review { .. } => "Review",
        }
    }

    /// Its stable id, when it has one.
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Input { id, .. } => Some(id),
            Self::Review { .. } => None,
        }
    }

    /// Whether it is the review step.
    pub fn is_review(&self) -> bool {
        matches!(self, Self::Review { .. })
    }
}

/// What a finished wizard will do. It is a plan, not an applied change.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Commit {
    wizard: RequiredWizard,
    changes: Vec<String>,
    failure_impact: String,
}

impl Commit {
    /// The wizard it came from.
    pub const fn wizard(&self) -> RequiredWizard {
        self.wizard
    }

    /// Every change it would make.
    pub fn changes(&self) -> &[String] {
        &self.changes
    }

    /// What a failure part-way would leave behind.
    pub fn failure_impact(&self) -> &str {
        &self.failure_impact
    }

    /// The commit in lines, for a caller that prints or logs it.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} ({}): {} change(s)",
            self.wizard.id(),
            self.wizard.title(),
            self.changes.len()
        )];
        for change in &self.changes {
            lines.push(format!("  {change}"));
        }
        lines.push(format!("on failure: {}", self.failure_impact()));
        lines
    }
}

/// What a wizard settled to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The review was finished; these are the changes it would make.
    Committed(Commit),
    /// It was cancelled; nothing it declared will be applied.
    Cancelled,
}

/// One wizard running in the shared shell.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Wizard {
    required: RequiredWizard,
    steps: Vec<Step>,
    current: usize,
    pending_changes: Vec<String>,
}

impl Wizard {
    /// A shell for one required wizard, with no steps yet.
    pub fn new(required: RequiredWizard) -> Self {
        Self {
            required,
            steps: Vec::new(),
            current: 0,
            pending_changes: Vec::new(),
        }
    }

    /// The wizard it is.
    pub const fn required(&self) -> RequiredWizard {
        self.required
    }

    /// Its steps, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The step it is on.
    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.current)
    }

    /// The zero-based position of the current step.
    pub const fn position(&self) -> usize {
        self.current
    }

    /// Whether the review step is current and no step follows it.
    pub fn is_on_review(&self) -> bool {
        matches!(self.current(), Some(step) if step.is_review())
            && self.current + 1 == self.steps.len()
    }

    /// Adds an input step. A step after the review is refused: the review is the last step.
    pub fn add_input(
        &mut self,
        id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<(), String> {
        if self.steps.iter().any(Step::is_review) {
            return Err("the review step is last; no step may follow it".to_owned());
        }
        self.steps.push(Step::input(id, title));
        Ok(())
    }

    /// Records one change the review will show. A change after the review is refused.
    pub fn add_change(&mut self, change: impl Into<String>) -> Result<(), String> {
        if self.steps.iter().any(Step::is_review) {
            return Err("the review step is last; no change may follow it".to_owned());
        }
        self.pending_changes.push(change.into());
        Ok(())
    }

    /// Adds the review step with every change recorded so far and the failure impact.
    ///
    /// A review with nothing to show is refused: the last step has to show the final change.
    pub fn add_review(&mut self, failure_impact: impl Into<String>) -> Result<(), String> {
        if self.steps.is_empty() {
            return Err("a wizard needs at least one input step before its review".to_owned());
        }
        if self.steps.iter().any(Step::is_review) {
            return Err("a wizard has exactly one review step".to_owned());
        }
        if self.pending_changes.is_empty() {
            return Err("the review has no change to show".to_owned());
        }
        let changes = std::mem::take(&mut self.pending_changes);
        self.steps.push(Step::review(changes, failure_impact));
        Ok(())
    }

    /// The current step's lines, including the whole change when it is the review.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} ({}): step {}/{}",
            self.required.id(),
            self.required.title(),
            self.current + 1,
            self.steps.len()
        )];
        match self.current() {
            Some(Step::Input { id, title }) => lines.push(format!("{id}: {title}")),
            Some(Step::Review {
                changes,
                failure_impact,
            }) => {
                lines.push("review:".to_owned());
                for change in changes {
                    lines.push(format!("  {change}"));
                }
                lines.push(format!("on failure: {failure_impact}"));
            }
            None => lines.push("no steps yet".to_owned()),
        }
        lines
    }

    /// Moves to the next step, refusing on the last one.
    pub fn next(&mut self) -> Result<&Step, String> {
        if self.current + 1 >= self.steps.len() {
            return Err("the wizard is on its last step; finish or cancel".to_owned());
        }
        self.current += 1;
        self.current()
            .ok_or_else(|| "the wizard has no current step".to_owned())
    }

    /// Moves to the previous step, refusing on the first one.
    pub fn back(&mut self) -> Result<&Step, String> {
        if self.current == 0 {
            return Err("the wizard is on its first step; there is no previous step".to_owned());
        }
        self.current -= 1;
        self.current()
            .ok_or_else(|| "the wizard has no current step".to_owned())
    }

    /// Finishes from the review. Anything else is refused, so a wizard cannot look finished early.
    pub fn finish(self) -> Result<Outcome, String> {
        let Some(Step::Review {
            changes,
            failure_impact,
        }) = self.current()
        else {
            return Err("the wizard can only finish on its review step".to_owned());
        };
        if self.current + 1 != self.steps.len() {
            return Err("the review is not the last step".to_owned());
        }
        let changes = changes.clone();
        let failure_impact = failure_impact.clone();
        Ok(Outcome::Committed(Commit {
            wizard: self.required,
            changes,
            failure_impact,
        }))
    }

    /// Cancels the wizard. Nothing it declared is applied, so no half project is left behind.
    pub fn cancel(self) -> Outcome {
        Outcome::Cancelled
    }
}

/// What a confirmation settled to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConfirmationOutcome {
    /// The user acknowledged the action.
    Acknowledged,
    /// The user declined it; nothing runs.
    Declined,
}

/// A confirmation the shell asks before a required action.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Confirmation {
    required: RequiredConfirmation,
    outcome: Option<ConfirmationOutcome>,
}

impl Confirmation {
    /// A confirmation that has not been answered.
    pub fn new(required: RequiredConfirmation) -> Self {
        Self {
            required,
            outcome: None,
        }
    }

    /// The confirmation it is.
    pub const fn required(&self) -> RequiredConfirmation {
        self.required
    }

    /// Whether it has been answered.
    pub const fn outcome(&self) -> Option<ConfirmationOutcome> {
        self.outcome
    }

    /// What it asks.
    pub fn ask(&self) -> String {
        format!("{}: {}", self.required.id(), self.required.title())
    }

    /// Acknowledges the action. An answered confirmation cannot be answered again.
    pub fn acknowledge(&mut self) -> Result<ConfirmationOutcome, String> {
        if self.outcome.is_some() {
            return Err(format!("{} is already answered", self.required.id()));
        }
        self.outcome = Some(ConfirmationOutcome::Acknowledged);
        Ok(ConfirmationOutcome::Acknowledged)
    }

    /// Declines the action. An answered confirmation cannot be answered again.
    pub fn decline(&mut self) -> Result<ConfirmationOutcome, String> {
        if self.outcome.is_some() {
            return Err(format!("{} is already answered", self.required.id()));
        }
        self.outcome = Some(ConfirmationOutcome::Declined);
        Ok(ConfirmationOutcome::Declined)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Confirmation, ConfirmationOutcome, Outcome, RequiredConfirmation, RequiredWizard, Step,
        Wizard,
    };

    fn an_import_wizard() -> Wizard {
        let mut wizard = Wizard::new(RequiredWizard::ImportProject);
        wizard
            .add_input("source", "Choose the project to import")
            .expect("an input step");
        wizard
            .add_input("options", "Choose what to import")
            .expect("an input step");
        wizard.add_change("copy src/").expect("a change");
        wizard.add_change("write rustage.json").expect("a change");
        wizard
            .add_review("the copy stops at the file that failed; the source is untouched")
            .expect("a review");
        wizard
    }

    #[test]
    fn the_required_lists_are_exactly_the_roadmaps() {
        assert_eq!(RequiredWizard::ALL.len(), 13);
        assert_eq!(RequiredConfirmation::ALL.len(), 8);
        for wizard in RequiredWizard::ALL {
            assert_eq!(RequiredWizard::from_id(wizard.id()), Some(wizard));
            assert!(!wizard.title().is_empty());
        }
        assert_eq!(RequiredWizard::from_id("wizard.unknown"), None);
        for confirmation in RequiredConfirmation::ALL {
            assert_eq!(
                RequiredConfirmation::from_id(confirmation.id()),
                Some(confirmation)
            );
            assert!(!confirmation.title().is_empty());
        }
        // The ids are unique, so a registry keyed by id cannot lose an entry.
        let mut ids: Vec<&str> = RequiredWizard::ALL
            .iter()
            .map(|wizard| wizard.id())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), RequiredWizard::ALL.len());
    }

    #[test]
    fn steps_walk_forward_and_back_without_running_past_either_end() {
        let mut wizard = an_import_wizard();
        assert_eq!(wizard.position(), 0);
        assert_eq!(wizard.steps().len(), 3);
        assert_eq!(
            wizard.next().expect("a step").title(),
            "Choose what to import"
        );
        assert_eq!(wizard.position(), 1);
        assert_eq!(wizard.next().expect("a step").title(), "Review");
        assert!(wizard.is_on_review());
        assert_eq!(
            wizard.next().expect_err("the last step"),
            "the wizard is on its last step; finish or cancel"
        );
        assert_eq!(
            wizard.back().expect("a step").title(),
            "Choose what to import"
        );
        assert_eq!(
            wizard.back().expect("a step").title(),
            "Choose the project to import"
        );
        assert_eq!(
            wizard.back().expect_err("the first step"),
            "the wizard is on its first step; there is no previous step"
        );
    }

    #[test]
    fn the_review_shows_the_whole_change_and_the_failure_impact() {
        let mut wizard = an_import_wizard();
        wizard.next().expect("a step");
        wizard.next().expect("a step");
        let lines = wizard.render();
        assert!(lines.iter().any(|line| line == "  copy src/"), "{lines:?}");
        assert!(
            lines.iter().any(|line| line
                == "on failure: the copy stops at the file that failed; the source is untouched"),
            "{lines:?}"
        );
        let Outcome::Committed(commit) = wizard.finish().expect("a commit") else {
            panic!("a finished wizard commits");
        };
        assert_eq!(commit.wizard(), RequiredWizard::ImportProject);
        assert_eq!(commit.changes().len(), 2);
        assert!(commit.failure_impact().contains("source is untouched"));
        assert_eq!(
            commit.lines().last().map(String::as_str),
            Some("on failure: the copy stops at the file that failed; the source is untouched")
        );
    }

    #[test]
    fn a_wizard_cannot_finish_before_its_review() {
        let wizard = an_import_wizard();
        assert_eq!(
            wizard.finish().expect_err("an early finish"),
            "the wizard can only finish on its review step"
        );
        // The refusal consumed the wizard; the same checks hold on a fresh one at the review.
        let mut wizard = an_import_wizard();
        wizard.next().expect("a step");
        assert!(wizard.finish().is_err());
    }

    #[test]
    fn a_cancel_applies_nothing_and_leaves_no_half_state() {
        let wizard = an_import_wizard();
        assert_eq!(wizard.cancel(), Outcome::Cancelled);
        // The shell holds the changes, so there is no place a half-applied project could be read from.
        let fresh = Wizard::new(RequiredWizard::NewProject);
        assert!(fresh.steps().is_empty());
        assert_eq!(fresh.cancel(), Outcome::Cancelled);
    }

    #[test]
    fn a_review_is_last_and_needs_something_to_show() {
        let mut wizard = Wizard::new(RequiredWizard::AddTarget);
        assert_eq!(
            wizard.add_review("nothing").expect_err("no steps"),
            "a wizard needs at least one input step before its review"
        );
        wizard
            .add_input("target", "Choose a target")
            .expect("a step");
        assert_eq!(
            wizard.add_review("nothing").expect_err("no changes"),
            "the review has no change to show"
        );
        wizard.add_change("add target desktop").expect("a change");
        wizard
            .add_review("the target list is unchanged")
            .expect("a review");
        assert_eq!(
            wizard.add_input("more", "More").expect_err("after review"),
            "the review step is last; no step may follow it"
        );
        assert_eq!(
            wizard.add_change("more").expect_err("after review"),
            "the review step is last; no change may follow it"
        );
        assert_eq!(
            wizard.add_review("again").expect_err("one review"),
            "a wizard has exactly one review step"
        );
    }

    #[test]
    fn a_confirmation_is_answered_once_and_declining_does_not_accept() {
        let mut confirmation = Confirmation::new(RequiredConfirmation::OverwriteExternal);
        assert_eq!(confirmation.outcome(), None);
        assert!(confirmation.ask().contains("confirm.overwrite-external"));
        assert_eq!(
            confirmation.decline().expect("a decline"),
            ConfirmationOutcome::Declined
        );
        assert_eq!(
            confirmation.acknowledge().expect_err("already answered"),
            "confirm.overwrite-external is already answered"
        );

        let mut confirmation = Confirmation::new(RequiredConfirmation::CloseDirtyProject);
        assert_eq!(
            confirmation.acknowledge().expect("an acknowledgement"),
            ConfirmationOutcome::Acknowledged
        );
        assert_eq!(
            confirmation.required(),
            RequiredConfirmation::CloseDirtyProject
        );
    }

    #[test]
    fn a_step_keeps_its_id_and_the_review_has_none() {
        let input = Step::input("source", "Choose the source");
        assert_eq!(input.id(), Some("source"));
        assert_eq!(input.title(), "Choose the source");
        assert!(!input.is_review());
        let review = Step::review(vec!["a".to_owned()], "nothing");
        assert_eq!(review.id(), None);
        assert_eq!(review.title(), "Review");
        assert!(review.is_review());
    }
}
