// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The preview viewport matrix: one shared document, one configuration and one running session per
//! viewport (task T14.04).
//!
//! The acceptance line is *"the viewports share the document and their running session state is
//! isolated"*. Both halves are structural, so the type makes both impossible to get wrong:
//!
//! * **There is one document, held by the matrix.** [`Matrix::revision`] is a single fact, so every
//!   viewport is a view of the same document by construction rather than by discipline.
//! * **A session belongs to its viewport.** [`Viewport::session`] is per viewport, so starting,
//!   stopping or restarting one cannot reach another — a test starts one and asserts the other is
//!   still stopped.
//! * **A configuration is per viewport.** Size, orientation, theme, language and sample are the
//!   viewport's own, so the same design is reviewed at several settings at once.
//! * **A name is unique.** [`Matrix::with`] refuses a second viewport of the same name, so a viewport
//!   is always addressable.

#![allow(dead_code)]

/// Which way round a viewport is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orientation {
    /// Taller than it is wide.
    Portrait,
    /// Wider than it is tall.
    Landscape,
}

impl Orientation {
    /// The orientation's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }

    /// Reads an orientation's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "portrait" => Some(Self::Portrait),
            "landscape" => Some(Self::Landscape),
            _ => None,
        }
    }
}

/// One viewport's running state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Session {
    /// Whether it is running.
    pub running: bool,
    /// How many times it has been rebuilt.
    pub restarts: u32,
}

/// One viewport: its configuration and its own session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Viewport {
    /// The name it is addressed by.
    pub name: String,
    /// Its size.
    pub size: (u32, u32),
    /// Which way round it is.
    pub orientation: Orientation,
    /// The theme it is previewed under.
    pub theme: Option<String>,
    /// The language it is previewed in.
    pub language: String,
    /// The sample data it is previewed with.
    pub sample: Option<String>,
    session: Session,
}

impl Viewport {
    /// A viewport with its configuration and a stopped session.
    pub fn new(name: impl Into<String>, size: (u32, u32), orientation: Orientation) -> Self {
        Self {
            name: name.into(),
            size,
            orientation,
            theme: None,
            language: "en".to_owned(),
            sample: None,
            session: Session::default(),
        }
    }

    /// The same viewport with a theme.
    pub fn with_theme(mut self, theme: impl Into<String>) -> Self {
        self.theme = Some(theme.into());
        self
    }

    /// The same viewport with a language.
    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = language.into();
        self
    }

    /// The same viewport with a sample dataset.
    pub fn with_sample(mut self, sample: impl Into<String>) -> Self {
        self.sample = Some(sample.into());
        self
    }

    /// Its session.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The viewport in one line.
    pub fn render(&self) -> String {
        format!(
            "{}: {}x{} {} theme={} language={} sample={} session={}",
            self.name,
            self.size.0,
            self.size.1,
            self.orientation.name(),
            self.theme.as_deref().unwrap_or("(none)"),
            self.language,
            self.sample.as_deref().unwrap_or("(none)"),
            if self.session.running {
                "running"
            } else {
                "stopped"
            }
        )
    }
}

/// The viewports a preview shows, over one document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Matrix {
    /// Which document they all show.
    document: String,
    /// The document's revision, shared by every viewport.
    revision: u32,
    viewports: Vec<Viewport>,
}

impl Matrix {
    /// A matrix over a document, with no viewports yet.
    pub fn new(document: impl Into<String>) -> Self {
        Self {
            document: document.into(),
            revision: 0,
            viewports: Vec::new(),
        }
    }

    /// The same matrix with one more viewport, refusing a duplicate name.
    pub fn with(mut self, viewport: Viewport) -> Result<Self, String> {
        if self.viewport(&viewport.name).is_some() {
            return Err(format!(
                "a viewport named {:?} already exists",
                viewport.name
            ));
        }
        self.viewports.push(viewport);
        Ok(self)
    }

    /// Adds a viewport, refusing a duplicate name.
    pub fn push(&mut self, viewport: Viewport) -> Result<(), String> {
        if self.viewport(&viewport.name).is_some() {
            return Err(format!(
                "a viewport named {:?} already exists",
                viewport.name
            ));
        }
        self.viewports.push(viewport);
        Ok(())
    }

    /// The document every viewport shows.
    pub fn document(&self) -> &str {
        &self.document
    }

    /// The document's revision, one fact for all viewports.
    pub fn revision(&self) -> u32 {
        self.revision
    }

    /// Commits a change to the document, moving the revision every viewport sees.
    pub fn commit(&mut self) {
        self.revision += 1;
    }

    /// A viewport by name.
    pub fn viewport(&self, name: &str) -> Option<&Viewport> {
        self.viewports.iter().find(|viewport| viewport.name == name)
    }

    /// A viewport by name, mutably.
    pub fn viewport_mut(&mut self, name: &str) -> Option<&mut Viewport> {
        self.viewports
            .iter_mut()
            .find(|viewport| viewport.name == name)
    }

    /// The viewports, in order.
    pub fn viewports(&self) -> &[Viewport] {
        &self.viewports
    }

    /// Starts one viewport's session.
    pub fn start(&mut self, name: &str) -> Result<(), String> {
        let viewport = self
            .viewport_mut(name)
            .ok_or_else(|| format!("no viewport named {name:?}"))?;
        viewport.session.running = true;
        Ok(())
    }

    /// Stops one viewport's session.
    pub fn stop(&mut self, name: &str) -> Result<(), String> {
        let viewport = self
            .viewport_mut(name)
            .ok_or_else(|| format!("no viewport named {name:?}"))?;
        viewport.session.running = false;
        Ok(())
    }

    /// Rebuilds one viewport, answering how many times it has been rebuilt.
    pub fn restart(&mut self, name: &str) -> Result<u32, String> {
        let viewport = self
            .viewport_mut(name)
            .ok_or_else(|| format!("no viewport named {name:?}"))?;
        viewport.session.restarts += 1;
        Ok(viewport.session.restarts)
    }

    /// Removes a viewport, leaving the others' sessions where they were.
    pub fn remove(&mut self, name: &str) -> Result<Viewport, String> {
        let index = self
            .viewports
            .iter()
            .position(|viewport| viewport.name == name)
            .ok_or_else(|| format!("no viewport named {name:?}"))?;
        Ok(self.viewports.remove(index))
    }

    /// The matrix in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "document: {} (revision {}); viewports: {}",
            self.document,
            self.revision,
            self.viewports.len()
        )];
        for viewport in &self.viewports {
            lines.push(format!("  {}", viewport.render()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{Matrix, Orientation, Viewport};

    fn viewport(name: &str) -> Viewport {
        Viewport::new(name, (390, 844), Orientation::Portrait)
    }

    #[test]
    fn every_viewport_shares_the_one_document() {
        let matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport")
            .with(Viewport::new("tablet", (1024, 768), Orientation::Landscape))
            .expect("a viewport");
        assert_eq!(matrix.document(), "design.json");
        assert_eq!(matrix.viewports().len(), 2);
    }

    #[test]
    fn committing_a_change_moves_the_revision_for_all() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport");
        assert_eq!(matrix.revision(), 0);
        matrix.commit();
        matrix.commit();
        assert_eq!(matrix.revision(), 2);
    }

    #[test]
    fn starting_one_viewport_does_not_start_another() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport")
            .with(viewport("tablet"))
            .expect("a viewport");
        matrix.start("phone").expect("a start");
        assert!(matrix.viewport("phone").expect("phone").session().running);
        assert!(!matrix.viewport("tablet").expect("tablet").session().running);
    }

    #[test]
    fn restarting_one_viewport_counts_only_on_that_one() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport")
            .with(viewport("tablet"))
            .expect("a viewport");
        assert_eq!(matrix.restart("phone").expect("a restart"), 1);
        assert_eq!(matrix.restart("phone").expect("a restart"), 2);
        assert_eq!(
            matrix
                .viewport("tablet")
                .expect("tablet")
                .session()
                .restarts,
            0
        );
    }

    #[test]
    fn stopping_one_viewport_leaves_another_running() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport")
            .with(viewport("tablet"))
            .expect("a viewport");
        matrix.start("phone").expect("a start");
        matrix.start("tablet").expect("a start");
        matrix.stop("phone").expect("a stop");
        assert!(!matrix.viewport("phone").expect("phone").session().running);
        assert!(matrix.viewport("tablet").expect("tablet").session().running);
    }

    #[test]
    fn viewports_can_carry_different_configurations() {
        let matrix = Matrix::new("design.json")
            .with(
                Viewport::new("phone", (390, 844), Orientation::Portrait)
                    .with_theme("dark")
                    .with_language("zh-CN")
                    .with_sample("empty"),
            )
            .expect("a viewport")
            .with(
                Viewport::new("tablet", (1024, 768), Orientation::Landscape)
                    .with_theme("light")
                    .with_sample("error"),
            )
            .expect("a viewport");
        let phone = matrix.viewport("phone").expect("phone");
        let tablet = matrix.viewport("tablet").expect("tablet");
        assert_eq!(phone.theme.as_deref(), Some("dark"));
        assert_eq!(phone.language, "zh-CN");
        assert_eq!(phone.sample.as_deref(), Some("empty"));
        assert_eq!(tablet.orientation, Orientation::Landscape);
        assert_eq!(tablet.language, "en");
        assert_eq!(tablet.sample.as_deref(), Some("error"));
    }

    #[test]
    fn a_duplicate_viewport_name_is_refused() {
        let matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport");
        let outcome = matrix.with(viewport("phone"));
        assert!(outcome.unwrap_err().contains("already exists"));
    }

    #[test]
    fn an_unknown_viewport_name_is_refused() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport");
        assert!(
            matrix
                .start("gone")
                .unwrap_err()
                .contains("no viewport named")
        );
        assert!(matrix.stop("gone").is_err());
        assert!(matrix.restart("gone").is_err());
        assert!(matrix.remove("gone").is_err());
    }

    #[test]
    fn removing_one_viewport_leaves_the_others_state_alone() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport")
            .with(viewport("tablet"))
            .expect("a viewport");
        matrix.start("tablet").expect("a start");
        let removed = matrix.remove("phone").expect("a remove");
        assert_eq!(removed.name, "phone");
        assert_eq!(matrix.viewports().len(), 1);
        assert!(matrix.viewport("tablet").expect("tablet").session().running);
    }

    #[test]
    fn the_orientation_parses_and_renders() {
        assert_eq!(Orientation::parse("portrait"), Some(Orientation::Portrait));
        assert_eq!(Orientation::parse("blob"), None);
        assert_eq!(Orientation::Landscape.name(), "landscape");
    }

    #[test]
    fn the_matrix_renders_the_document_and_the_viewports() {
        let mut matrix = Matrix::new("design.json")
            .with(viewport("phone"))
            .expect("a viewport");
        matrix.commit();
        let lines = matrix.render();
        assert_eq!(lines[0], "document: design.json (revision 1); viewports: 1");
        assert!(lines[1].contains("phone: 390x844 portrait"));
    }
}
