// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The configuration a build or a run is decided by: which target is active, what it inherits, and
//! where every value came from (task T02.06).
//!
//! The acceptance line is two sentences — **the source is visible**, and **an active configuration
//! does not leak between projects** — and both are properties of the types rather than rules a
//! caller has to remember.
//!
//! ## The source is visible
//!
//! A value is never just a value. [`Setting`] carries the layer it came from ([`Source`]) and the
//! values it replaced ([`Setting::replaced`]), so "release, from the target the person chose, which
//! replaced the built-in `debug`" is what a caller has, and there is no way to read the value
//! without also having the source. A layer that says nothing is not a layer that said "no": the
//! project document's target either names a profile or is silent, and silence means the layer below
//! decides.
//!
//! ## The layers
//!
//! Bottom to top: this build's own default (Cargo's own spelling of "no choice": `debug`, no
//! features), the project document (the chosen target), and this machine's overrides. There is
//! deliberately **no project-level profile member**: version 2 of the format does not have one, and
//! adding one would be a format change (decisions `F1`/`Q6`) that this task does not need. A target
//! that is silent about a setting therefore inherits the built-in default, and the report says so
//! instead of implying the project said something.
//!
//! A layer that names features **replaces** the list; it does not add to it. Cargo's own feature
//! selection is additive, but this is a *target configuration* the designer owns, and a list that
//! replaced another one is reported ([`Setting::replaced`]) so a removed feature cannot look like one
//! that was never asked for.
//!
//! ## An active configuration does not leak between projects
//!
//! The active target and this machine's overrides are machine-local state: they live in the host's
//! configuration directory, not in a project document, because they are true of this machine and
//! would be wrong in a file that gets committed. Every entry in that file therefore carries the
//! **identity** of the project it was recorded for, and every read takes the identity it is asked
//! about:
//!
//! * [`MachineConfiguration::active_for`] answers for one project, and a choice recorded for another
//!   project is [`ActiveVerdict::AnotherProject`], which the resolution does not use — it is reported
//!   with both identities named instead;
//! * an override recorded for another project, or for another target, is [`Skipped`] with the reason,
//!   never applied quietly;
//! * a resolved configuration carries the identity it was resolved for, and
//!   [`ResolvedConfiguration::for_project`] refuses to hand it to anybody else.
//!
//! ## Credentials are references, and the view says only that
//!
//! [`credentials`] turns the services a project declares into [`CredentialView`]s: the name a person
//! sees, where the material lives, and the health the host's credential store reports
//! ([`CredentialHealth`]). The material itself is never read, never held and never rendered here —
//! `lookup` is what the network layer calls when it is about to make a request, and nothing in this
//! module needs the value to answer "which credential is this and is it there?".
//!
//! ## What is not here
//!
//! The toolchain, the SDK, the debugger and the device are a *kit*, and decision `T02.10` keeps a kit
//! separate from the output profile and the target triple; this module knows nothing about kits.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::contracts::revision::FormatVersion;
use crate::host::network::{CredentialHealth, CredentialSource, CredentialStore, ServicePolicy};
use crate::project::record::format::{ProjectDocument, ProjectError, format_version};
use crate::project::record::json::{self, Json, JsonKind, JsonObject, Number};

/// The configuration file format this build writes.
///
/// Its own sequence, like the recent list's: this file is machine-local state, and it must not change
/// meaning because a project document gained a member.
pub const CONFIGURATION_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The member names a configuration file version 1 defines, in the order the writer writes them.
pub const CONFIGURATION_MEMBERS: [&str; 3] = ["format", "active", "overrides"];

/// The member names an active choice version 1 defines, in the order the writer writes them.
pub const ACTIVE_MEMBERS: [&str; 5] = ["project", "name", "target", "chosen_at_unix_ms", "unknown"];

/// The member names an override version 1 defines, in the order the writer writes them.
pub const OVERRIDE_MEMBERS: [&str; 5] =
    ["project", "target", "key", "value", "recorded_at_unix_ms"];

/// The profile a build uses when nothing else says: Cargo's own default, named once here.
pub const DEFAULT_PROFILE: &str = "debug";

/// One setting a build or a run is decided by.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SettingKey {
    /// Which build profile the target uses.
    Profile,
    /// Which features the target is built with.
    Features,
    /// A key this build does not know, kept so that a newer build's override is not silently dropped
    /// by an older one. It is always skipped, with the reason.
    Unrecognised(String),
}

impl SettingKey {
    /// The word this key is written as.
    pub fn name(&self) -> &str {
        match self {
            Self::Profile => "profile",
            Self::Features => "features",
            Self::Unrecognised(word) => word,
        }
    }

    /// Reads a key, or `None` when nothing can be a key at all (an empty word).
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "" => None,
            "profile" => Some(Self::Profile),
            "features" => Some(Self::Features),
            other => Some(Self::Unrecognised(other.to_owned())),
        }
    }
}

/// The value of a setting.
///
/// Two shapes, because two settings have two shapes: a profile is a word and features are a list.
/// Keeping them apart is what makes "an override for `features` that gives a word" a refusal rather
/// than a value somebody has to interpret.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SettingValue {
    /// One word, for a setting that holds one.
    Word(String),
    /// A list, for a setting that holds several.
    List(Vec<String>),
}

impl SettingValue {
    /// A list from words, trimming and dropping the empty ones.
    pub fn list(words: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::List(
            words
                .into_iter()
                .map(Into::into)
                .map(|word: String| word.trim().to_owned())
                .filter(|word| !word.is_empty())
                .collect(),
        )
    }

    /// The words, whichever shape it has.
    pub fn words(&self) -> Vec<&str> {
        match self {
            Self::Word(word) => vec![word.as_str()],
            Self::List(words) => words.iter().map(String::as_str).collect(),
        }
    }

    /// Whether this shape fits a setting.
    pub const fn fits(&self, key: &SettingKey) -> bool {
        matches!(
            (key, self),
            (SettingKey::Profile, Self::Word(_))
                | (SettingKey::Features, Self::List(_))
                | (SettingKey::Unrecognised(_), _)
        )
    }

    /// The shape's name, for a refusal.
    pub const fn shape(&self) -> &'static str {
        match self {
            Self::Word(_) => "a word",
            Self::List(_) => "a list",
        }
    }

    /// The value as a person reads it.
    pub fn render(&self) -> String {
        match self {
            Self::Word(word) => word.clone(),
            Self::List(words) if words.is_empty() => "no features".to_owned(),
            Self::List(words) => words.join(", "),
        }
    }
}

/// Where a value came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Source {
    /// This build's own default, because no layer said anything.
    BuiltIn,
    /// The target the person chose, as the project document declares it.
    Target {
        /// The target's name.
        name: String,
    },
    /// An override recorded on this machine, for one project — and for one target, or for every
    /// target when it names none.
    Override {
        /// The project it was recorded for.
        project: String,
        /// The target it was recorded for, when it named one.
        target: Option<String>,
    },
}

impl Source {
    /// The source as a person reads it.
    pub fn render(&self) -> String {
        match self {
            Self::BuiltIn => "the built-in default".to_owned(),
            Self::Target { name } => format!("target `{name}`"),
            Self::Override {
                target: Some(target),
                ..
            } => format!("an override for target `{target}`"),
            Self::Override { target: None, .. } => "an override for every target".to_owned(),
        }
    }
}

/// One resolved setting: the value, where it came from, and what it replaced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Setting {
    key: SettingKey,
    value: SettingValue,
    source: Source,
    replaced: Vec<String>,
}

impl Setting {
    /// Which setting this is.
    pub fn key(&self) -> &SettingKey {
        &self.key
    }

    /// The value that applies.
    pub fn value(&self) -> &SettingValue {
        &self.value
    }

    /// Where it came from.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// The values it replaced, each with the layer that gave it. Empty when nothing was replaced.
    pub fn replaced(&self) -> &[String] {
        &self.replaced
    }

    /// The setting in one line, source and all.
    pub fn render(&self) -> String {
        if self.replaced.is_empty() {
            return format!(
                "{}: {} (from {})",
                self.key.name(),
                self.value.render(),
                self.source.render()
            );
        }
        format!(
            "{}: {} (from {}; it replaced {})",
            self.key.name(),
            self.value.render(),
            self.source.render(),
            self.replaced.join(", ")
        )
    }
}

/// An override recorded on this machine, for one project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Override {
    project: String,
    target: Option<String>,
    key: SettingKey,
    value: SettingValue,
    recorded_at_millis: i64,
}

impl Override {
    /// An override for a project's setting, refused when the value's shape does not fit the setting.
    pub fn new(
        project: impl Into<String>,
        target: Option<String>,
        key: SettingKey,
        value: SettingValue,
        at: SystemTime,
    ) -> Result<Self, ConfigurationRefusal> {
        if !value.fits(&key) {
            return Err(ConfigurationRefusal::WrongShape {
                key: key.name().to_owned(),
                shape: value.shape(),
            });
        }
        Ok(Self {
            project: project.into(),
            target: target.map(|target| target.trim().to_owned()),
            key,
            value,
            recorded_at_millis: crate::project::record::format::unix_millis(at).unwrap_or_default(),
        })
    }

    /// The project it was recorded for.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The target it was recorded for, when it named one.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// Which setting it overrides.
    pub fn key(&self) -> &SettingKey {
        &self.key
    }

    /// The value it gives.
    pub fn value(&self) -> &SettingValue {
        &self.value
    }

    /// When it was recorded.
    pub const fn recorded_at_millis(&self) -> i64 {
        self.recorded_at_millis
    }

    /// Whether this override applies to a target: it applies to every target when it names none.
    pub fn applies_to(&self, target: &str) -> bool {
        self.target.as_deref().is_none_or(|named| named == target)
    }
}

/// Why an override was not applied.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SkipReason {
    /// It belongs to another project.
    AnotherProject {
        /// The project it was recorded for.
        recorded: String,
    },
    /// It belongs to another target than the one that is active.
    AnotherTarget {
        /// The target it was recorded for.
        recorded: String,
        /// The target that is active.
        active: String,
    },
    /// This build does not know the setting.
    Unrecognised {
        /// The word in the file.
        key: String,
    },
}

impl SkipReason {
    /// The reason in one line.
    pub fn render(&self) -> String {
        match self {
            Self::AnotherProject { recorded } => {
                format!("it was recorded for another project ({recorded})")
            }
            Self::AnotherTarget { recorded, active } => {
                format!("it was recorded for target `{recorded}`, and `{active}` is active")
            }
            Self::Unrecognised { key } => {
                format!("this build does not know the setting `{key}`, so it was not used")
            }
        }
    }
}

/// An override that was not applied, and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Skipped {
    entry: Override,
    reason: SkipReason,
}

impl Skipped {
    /// The override.
    pub fn entry(&self) -> &Override {
        &self.entry
    }

    /// Why it was not applied.
    pub fn reason(&self) -> &SkipReason {
        &self.reason
    }
}

/// The target a project was last told to build, on this machine.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ActiveChoice {
    project: String,
    name: String,
    target: String,
    chosen_at_millis: i64,
}

impl ActiveChoice {
    /// A choice, made now.
    pub fn new(
        project: impl Into<String>,
        name: impl Into<String>,
        target: impl Into<String>,
        at: SystemTime,
    ) -> Self {
        Self {
            project: project.into(),
            name: name.into(),
            target: target.into(),
            chosen_at_millis: crate::project::record::format::unix_millis(at).unwrap_or_default(),
        }
    }

    /// The project it belongs to.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The project's name, which is what a person recognises it by.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The target it chose.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// When it was made.
    pub const fn chosen_at_millis(&self) -> i64 {
        self.chosen_at_millis
    }
}

/// What this machine last chose for a project.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ActiveVerdict<'a> {
    /// A target was chosen for this project.
    Chosen(&'a ActiveChoice),
    /// A target was chosen for a **different** project, and it is not this project's. The choice is
    /// reported rather than used: that is the whole point of the identity in the file.
    AnotherProject(&'a ActiveChoice),
    /// Nothing was ever chosen for this project.
    NeverChosen,
}

impl ActiveVerdict<'_> {
    /// The choice this verdict is about, whichever project it belongs to.
    pub const fn choice(&self) -> Option<&ActiveChoice> {
        match self {
            Self::Chosen(choice) | Self::AnotherProject(choice) => Some(choice),
            Self::NeverChosen => None,
        }
    }

    /// Whether the verdict can decide a build for the project it was asked about.
    pub const fn is_for_this_project(&self) -> bool {
        matches!(self, Self::Chosen(_))
    }
}

/// This machine's configuration state: the choices and the overrides.
///
/// It is machine-local, so it does not live in a project document; it is written the way a project
/// document is — a temporary file first, then a rename — and it needs no permission, because it is
/// the host's own state and not the project's (the same rule the recent list follows).
#[derive(Clone, Debug)]
pub struct MachineConfiguration {
    path: PathBuf,
    active: Vec<ActiveChoice>,
    overrides: Vec<Override>,
    capacity: usize,
}

impl MachineConfiguration {
    /// How many projects' choices the file keeps. Older ones fall off the end.
    pub const CAPACITY: usize = 10;

    /// An empty state that would be saved at a path.
    pub fn empty(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            active: Vec::new(),
            overrides: Vec::new(),
            capacity: Self::CAPACITY,
        }
    }

    /// A state with a different bound, which is what a test uses.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self.active.truncate(self.capacity);
        self
    }

    /// Reads the state, or an empty one when the file is not there yet.
    ///
    /// A file that is there and cannot be read is an error rather than an empty state: quietly
    /// forgetting which target a project builds because one file went wrong is a loss nobody sees.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let mut state = Self::empty(path.clone());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(state),
            Err(error) => {
                return Err(ProjectError::Io {
                    path: path.clone(),
                    action: "reading",
                    reason: error.to_string(),
                });
            }
        };
        let value = json::parse(&text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let version = format_version(&value)?;
        match version.get().cmp(&CONFIGURATION_FORMAT.get()) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: CONFIGURATION_FORMAT,
                });
            }
            std::cmp::Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: CONFIGURATION_FORMAT,
                });
            }
        }
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "the document".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        for member in ["active", "overrides"] {
            let Some(values) = object.get(member).and_then(Json::as_array) else {
                return Err(ProjectError::WrongType {
                    path: member.to_owned(),
                    expected: "an array",
                    found: object.get(member).map(Json::kind).unwrap_or(JsonKind::Null),
                });
            };
            for (index, value) in values.iter().enumerate() {
                if member == "active" {
                    state.active.push(decode_active(index, value)?);
                } else {
                    state.overrides.push(decode_override(index, value)?);
                }
            }
        }
        state.active.truncate(state.capacity);
        Ok(state)
    }

    /// Writes the state, through a temporary file and a rename.
    pub fn save(&self) -> Result<PathBuf, ProjectError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| ProjectError::Io {
                path: parent.to_path_buf(),
                action: "creating the directory this machine's configuration lives in",
                reason: error.to_string(),
            })?;
        }
        json::write_atomically(&self.path, &self.to_json().render()).map_err(|error| {
            ProjectError::Io {
                path: self.path.clone(),
                action: "writing",
                reason: error.to_string(),
            }
        })
    }

    /// Where the state is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What was chosen for a project, if anything was.
    pub fn active_for(&self, project: &str) -> ActiveVerdict<'_> {
        match self
            .active
            .iter()
            .find(|choice| choice.project == project)
            .map(ActiveVerdict::Chosen)
        {
            Some(verdict) => verdict,
            None => self
                .active
                .first()
                .map(ActiveVerdict::AnotherProject)
                .unwrap_or(ActiveVerdict::NeverChosen),
        }
    }

    /// Records a choice for a project, replacing the earlier one for the same project.
    pub fn set_active(
        &mut self,
        project: impl Into<String>,
        name: impl Into<String>,
        target: impl Into<String>,
        at: SystemTime,
    ) {
        let choice = ActiveChoice::new(project, name, target, at);
        self.active
            .retain(|existing| existing.project != choice.project);
        self.active.insert(0, choice);
        self.active.truncate(self.capacity);
    }

    /// Records an override, replacing an earlier one for the same project, target and setting.
    pub fn remember(&mut self, entry: Override) {
        self.overrides.retain(|existing| {
            !(existing.project == entry.project
                && existing.target == entry.target
                && existing.key == entry.key)
        });
        self.overrides.push(entry);
    }

    /// Forgets everything recorded for a project: its choice and its overrides.
    pub fn forget(&mut self, project: &str) -> usize {
        let before = self.active.len() + self.overrides.len();
        self.active.retain(|choice| choice.project != project);
        self.overrides.retain(|entry| entry.project != project);
        before - (self.active.len() + self.overrides.len())
    }

    /// The overrides that apply to a project and a target, and the ones that do not, with the reason.
    ///
    /// The identity is an argument rather than something the entry is trusted to carry: an override
    /// recorded for another project cannot be read out of here without also reading why it was
    /// skipped.
    pub fn overrides_for(&self, project: &str, target: &str) -> (Vec<&Override>, Vec<Skipped>) {
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        for entry in &self.overrides {
            let reason = if entry.project != project {
                Some(SkipReason::AnotherProject {
                    recorded: format!("{} ({})", entry.project, self.name_of(&entry.project)),
                })
            } else if !entry.applies_to(target) {
                Some(SkipReason::AnotherTarget {
                    recorded: entry
                        .target
                        .clone()
                        .unwrap_or_else(|| "every target".to_owned()),
                    active: target.to_owned(),
                })
            } else if matches!(entry.key, SettingKey::Unrecognised(_)) {
                Some(SkipReason::Unrecognised {
                    key: entry.key.name().to_owned(),
                })
            } else {
                None
            };
            match reason {
                Some(reason) => skipped.push(Skipped {
                    entry: entry.clone(),
                    reason,
                }),
                None => applied.push(entry),
            }
        }
        (applied, skipped)
    }

    /// The name recorded for a project, which is what a person recognises it by.
    fn name_of(&self, project: &str) -> String {
        self.active
            .iter()
            .find(|choice| choice.project == project)
            .map(|choice| choice.name.clone())
            .or_else(|| {
                self.overrides
                    .iter()
                    .find(|entry| entry.project == project)
                    .map(|entry| entry.project.clone())
            })
            .unwrap_or_else(|| "a project this file does not name".to_owned())
    }

    /// The choices, which is what a report shows.
    pub fn choices(&self) -> &[ActiveChoice] {
        &self.active
    }

    /// The overrides, which is what a report shows.
    pub fn overrides(&self) -> &[Override] {
        &self.overrides
    }

    /// The state as JSON.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert(
            "format",
            Json::number(u64::from(CONFIGURATION_FORMAT.get())),
        );
        out.insert(
            "active",
            Json::array(self.active.iter().map(encode_active).collect::<Vec<_>>()),
        );
        out.insert(
            "overrides",
            Json::array(
                self.overrides
                    .iter()
                    .map(encode_override)
                    .collect::<Vec<_>>(),
            ),
        );
        Json::Object(out)
    }

    /// One line per recorded thing, for a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.active.is_empty() && self.overrides.is_empty() {
            lines.push("nothing is recorded for any project on this machine".to_owned());
            return lines;
        }
        for choice in &self.active {
            lines.push(format!(
                "target `{}` for {} ({})",
                choice.target, choice.name, choice.project
            ));
        }
        for entry in &self.overrides {
            lines.push(format!(
                "{}: {} for {} ({})",
                entry.key.name(),
                entry.value.render(),
                entry.project,
                match entry.target.as_deref() {
                    Some(target) => format!("target `{target}`"),
                    None => "every target".to_owned(),
                }
            ));
        }
        lines
    }
}

/// Why a configuration could not be resolved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ConfigurationRefusal {
    /// The document declares no target, so there is nothing to build.
    NoTargets,
    /// The target that was asked for is not one the document declares.
    NoSuchTarget {
        /// What was asked for.
        asked: String,
        /// What the document declares.
        known: Vec<String>,
    },
    /// A target that was chosen for this project is no longer one of its targets.
    ///
    /// Refused rather than replaced: quietly building a target nobody chose is how a stale choice
    /// ends up looking like a current one.
    StaleChoice {
        /// The target the choice names.
        chosen: String,
        /// What the document declares.
        known: Vec<String>,
    },
    /// A value's shape does not fit the setting.
    WrongShape {
        /// The setting.
        key: String,
        /// The shape the value has.
        shape: &'static str,
    },
    /// A resolved configuration was asked about a different project.
    AnotherProject {
        /// The project it was resolved for.
        resolved: String,
        /// The project it was asked about.
        asked: String,
    },
}

impl ConfigurationRefusal {
    /// The refusal in one line.
    pub fn message(&self) -> String {
        match self {
            Self::NoTargets => {
                "this project declares no target, so there is nothing to build; add one first"
                    .to_owned()
            }
            Self::NoSuchTarget { asked, known } => format!(
                "this project has no target `{asked}`; it has {}",
                describe_list(known)
            ),
            Self::StaleChoice { chosen, known } => format!(
                "the target chosen for this project last time (`{chosen}`) is not one of its \
                 targets any more; it has {}, so this build does not silently build another one",
                describe_list(known)
            ),
            Self::WrongShape { key, shape } => {
                format!("`{key}` takes a different shape than {shape}")
            }
            Self::AnotherProject { resolved, asked } => format!(
                "this configuration was resolved for project {resolved}, and it was asked about \
                 {asked}; a configuration is not shared between projects"
            ),
        }
    }
}

/// How the target that was resolved for was arrived at.
///
/// The target is a value like any other, so it says where it came from too: a report that shows a
/// target without saying whether a person asked for it, this machine chose it last time, or it is
/// simply the project's first one would leave the most important decision unsourced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TargetSource {
    /// It was asked for on the command line.
    Asked,
    /// This machine chose it for this project before.
    ChosenOnThisMachine,
    /// It is the first target the project declares.
    FirstInTheDocument,
}

impl TargetSource {
    /// The source in a person's words.
    pub const fn render(&self) -> &'static str {
        match self {
            Self::Asked => "asked for on the command line",
            Self::ChosenOnThisMachine => "the target chosen for this project on this machine",
            Self::FirstInTheDocument => "this project's first target",
        }
    }
}

/// A target's configuration, resolved, with where every value came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResolvedConfiguration {
    project: String,
    name: String,
    target: String,
    target_source: TargetSource,
    settings: Vec<Setting>,
    applied: Vec<Override>,
    skipped: Vec<Skipped>,
}

impl ResolvedConfiguration {
    /// Resolves a project's configuration for a target.
    ///
    /// The target is the one asked for, then the one this machine chose for this project, then the
    /// document's first target. A choice recorded for another project is never used; it is reported
    /// as [`ActiveVerdict::AnotherProject`] and the project's own default decides.
    pub fn of(
        document: &ProjectDocument,
        requested: Option<&str>,
        machine: &MachineConfiguration,
    ) -> Result<Self, ConfigurationRefusal> {
        let identity = document.identity();
        let known: Vec<String> = document
            .targets()
            .iter()
            .map(|target| target.name().to_owned())
            .collect();
        let Some(first) = document.targets().first() else {
            return Err(ConfigurationRefusal::NoTargets);
        };
        let mut target_source = TargetSource::FirstInTheDocument;
        let target = match requested {
            Some(asked) => {
                let found = document
                    .targets()
                    .iter()
                    .find(|target| target.name() == asked)
                    .ok_or_else(|| ConfigurationRefusal::NoSuchTarget {
                        asked: asked.to_owned(),
                        known: known.clone(),
                    })?;
                target_source = TargetSource::Asked;
                found
            }
            None => match machine.active_for(identity.id()) {
                ActiveVerdict::Chosen(choice) => {
                    target_source = TargetSource::ChosenOnThisMachine;
                    document
                        .targets()
                        .iter()
                        .find(|target| target.name() == choice.target())
                        .ok_or_else(|| ConfigurationRefusal::StaleChoice {
                            chosen: choice.target().to_owned(),
                            known: known.clone(),
                        })?
                }
                // The recorded choice belongs to another project, or there is none: this project
                // decides for itself, and the report says the other choice was not used.
                ActiveVerdict::AnotherProject(_) | ActiveVerdict::NeverChosen => first,
            },
        };

        let (applied, skipped) = machine.overrides_for(identity.id(), target.name());
        let mut settings = vec![
            Setting {
                key: SettingKey::Profile,
                value: SettingValue::Word(DEFAULT_PROFILE.to_owned()),
                source: Source::BuiltIn,
                replaced: Vec::new(),
            },
            Setting {
                key: SettingKey::Features,
                value: SettingValue::List(Vec::new()),
                source: Source::BuiltIn,
                replaced: Vec::new(),
            },
        ];
        // The target's own words come next, then this machine's overrides, so the last writer of a
        // setting is the one that applies and every earlier value is recorded as replaced.
        if let Some(profile) = target.profile() {
            apply(
                &mut settings,
                &SettingKey::Profile,
                SettingValue::Word(profile.to_owned()),
                Source::Target {
                    name: target.name().to_owned(),
                },
            );
        }
        if !target.features().is_empty() {
            apply(
                &mut settings,
                &SettingKey::Features,
                SettingValue::List(target.features().to_vec()),
                Source::Target {
                    name: target.name().to_owned(),
                },
            );
        }
        for entry in &applied {
            apply(
                &mut settings,
                entry.key(),
                entry.value().clone(),
                Source::Override {
                    project: entry.project().to_owned(),
                    target: entry.target().map(str::to_owned),
                },
            );
        }

        Ok(Self {
            project: identity.id().to_owned(),
            name: identity.name().to_owned(),
            target: target.name().to_owned(),
            target_source,
            settings,
            applied: applied.into_iter().cloned().collect(),
            skipped,
        })
    }

    /// The project this configuration belongs to.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The project's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The target it was resolved for.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// How that target was arrived at.
    pub fn target_source(&self) -> &TargetSource {
        &self.target_source
    }

    /// The resolved settings, in a fixed order.
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// The overrides that applied.
    pub fn applied(&self) -> &[Override] {
        &self.applied
    }

    /// The overrides that did not, and why.
    pub fn skipped(&self) -> &[Skipped] {
        &self.skipped
    }

    /// One setting.
    pub fn setting(&self, key: &SettingKey) -> Option<&Setting> {
        let name = key.name();
        self.settings
            .iter()
            .find(|setting| setting.key.name() == name)
    }

    /// The configuration as a person reads it, or a refusal when it belongs to somebody else.
    ///
    /// This is the check the acceptance line names: a caller that holds a configuration it did not
    /// resolve has to say which project it is for, and this is what answers.
    pub fn for_project<'a>(&'a self, project: &str) -> Result<&'a Self, ConfigurationRefusal> {
        if self.project == project {
            return Ok(self);
        }
        Err(ConfigurationRefusal::AnotherProject {
            resolved: format!("{} ({})", self.project, self.name),
            asked: project.to_owned(),
        })
    }

    /// The configuration in a person's words, one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "{} ({}): target `{}` ({})",
            self.name,
            self.project,
            self.target,
            self.target_source.render()
        )];
        for setting in &self.settings {
            lines.push(format!("  {}", setting.render()));
        }
        for entry in &self.applied {
            lines.push(format!(
                "  applied override: {}: {} ({})",
                entry.key.name(),
                entry.value.render(),
                match entry.target.as_deref() {
                    Some(target) => format!("recorded for target `{target}`"),
                    None => "recorded for every target".to_owned(),
                }
            ));
        }
        for skipped in &self.skipped {
            lines.push(format!(
                "  not applied: {}: {} — {}",
                skipped.entry.key.name(),
                skipped.entry.value.render(),
                skipped.reason.render()
            ));
        }
        lines
    }
}

/// Puts a value in a setting and records what it replaced.
fn apply(settings: &mut [Setting], key: &SettingKey, value: SettingValue, source: Source) {
    let Some(setting) = settings
        .iter_mut()
        .find(|setting| setting.key.name() == key.name())
    else {
        return;
    };
    setting.replaced.push(format!(
        "`{}` from {}",
        setting.value.render(),
        setting.source.render()
    ));
    setting.value = value;
    setting.source = source;
}

/// One credential a project's services use, as a person sees it.
///
/// Never the material: a view holds the name, where it lives, and what the store says about it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CredentialView {
    service: String,
    name: String,
    source: CredentialSource,
    health: CredentialHealth,
}

impl CredentialView {
    /// The service that uses it.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The name a person sees and a keyring addresses.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Where the material lives.
    pub fn source(&self) -> &CredentialSource {
        &self.source
    }

    /// What the store says about it.
    pub fn health(&self) -> &CredentialHealth {
        &self.health
    }

    /// The credential in one line, with what to do about it when there is something to do.
    ///
    /// The next step is left out when it only repeats the reason: "missing: the TOKEN environment
    /// variable is not set — add the credential: the TOKEN environment variable is not set" says one
    /// thing twice, and a report that repeats itself is harder to read than one that does not.
    pub fn render(&self) -> String {
        let message = self.health.message();
        let mut line = format!(
            "{}: `{}` from {}: {message}",
            self.service,
            self.name,
            self.source.render()
        );
        if let Some(next) = self.health.next_step()
            && !repeats(&message, &next)
        {
            line.push_str(&format!(" — {next}"));
        }
        line
    }
}

/// The credentials a project's services use, with what the store says about each.
///
/// The store is an argument so that a test can answer without a keyring and without a token, and so
/// that this module never reads the material itself.
pub fn credentials(
    document: &ProjectDocument,
    store: &dyn CredentialStore,
    now: SystemTime,
) -> Vec<CredentialView> {
    document
        .services()
        .iter()
        .filter_map(|service| {
            let policy: &ServicePolicy = service.policy();
            let credential = policy.credential()?;
            Some(CredentialView {
                service: policy.service().to_owned(),
                name: credential.name().to_owned(),
                source: credential.source().clone(),
                health: credential.health(store, now),
            })
        })
        .collect()
}

/// Whether a next step only repeats what the reason already said.
fn repeats(message: &str, next: &str) -> bool {
    match message.split_once(": ") {
        Some((_, reason)) => next.contains(reason),
        None => false,
    }
}

/// A list in a person's words: `a`, `a` and `b`, `a`, `b` and `c`.
fn describe_list(words: &[String]) -> String {
    match words {
        [] => "none".to_owned(),
        [one] => format!("`{one}`"),
        [rest @ .., last] => format!(
            "{} and `{last}`",
            rest.iter()
                .map(|word| format!("`{word}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn encode_active(choice: &ActiveChoice) -> Json {
    let mut out = JsonObject::new();
    out.insert("project", Json::string(&choice.project));
    out.insert("name", Json::string(&choice.name));
    out.insert("target", Json::string(&choice.target));
    out.insert(
        "chosen_at_unix_ms",
        Json::Number(Number::from_i64(choice.chosen_at_millis)),
    );
    out.insert("unknown", Json::array(Vec::new()));
    Json::Object(out)
}

fn encode_override(entry: &Override) -> Json {
    let mut out = JsonObject::new();
    out.insert("project", Json::string(&entry.project));
    out.insert(
        "target",
        match entry.target.as_deref() {
            Some(target) => Json::string(target),
            None => Json::Null,
        },
    );
    out.insert("key", Json::string(entry.key.name()));
    out.insert("value", encode_value(&entry.value));
    out.insert(
        "recorded_at_unix_ms",
        Json::Number(Number::from_i64(entry.recorded_at_millis)),
    );
    Json::Object(out)
}

fn encode_value(value: &SettingValue) -> Json {
    match value {
        SettingValue::Word(word) => Json::string(word),
        SettingValue::List(words) => {
            Json::array(words.iter().map(Json::string).collect::<Vec<_>>())
        }
    }
}

fn decode_value(path: &str, value: &Json) -> Result<SettingValue, ProjectError> {
    match value {
        Json::String(word) => Ok(SettingValue::Word(word.clone())),
        Json::Array(values) => {
            let mut words = Vec::new();
            for (index, value) in values.iter().enumerate() {
                let Some(word) = value.as_str() else {
                    return Err(ProjectError::WrongType {
                        path: format!("{path}[{index}]"),
                        expected: "a string",
                        found: value.kind(),
                    });
                };
                words.push(word.to_owned());
            }
            Ok(SettingValue::List(words))
        }
        other => Err(ProjectError::WrongType {
            path: path.to_owned(),
            expected: "a word or a list of words",
            found: other.kind(),
        }),
    }
}

fn decode_active(index: usize, value: &Json) -> Result<ActiveChoice, ProjectError> {
    let Some(object) = value.as_object() else {
        return Err(ProjectError::WrongType {
            path: format!("active[{index}]"),
            expected: "an object",
            found: value.kind(),
        });
    };
    let text = |name: &str| -> Result<String, ProjectError> {
        object
            .get(name)
            .and_then(Json::as_str)
            .map(str::to_owned)
            .ok_or(ProjectError::WrongType {
                path: format!("active[{index}].{name}"),
                expected: "a string",
                found: object.get(name).map(Json::kind).unwrap_or(JsonKind::Null),
            })
    };
    Ok(ActiveChoice {
        project: text("project")?,
        name: text("name")?,
        target: text("target")?,
        chosen_at_millis: object
            .get("chosen_at_unix_ms")
            .and_then(Json::as_i64)
            .unwrap_or_default(),
    })
}

fn decode_override(index: usize, value: &Json) -> Result<Override, ProjectError> {
    let Some(object) = value.as_object() else {
        return Err(ProjectError::WrongType {
            path: format!("overrides[{index}]"),
            expected: "an object",
            found: value.kind(),
        });
    };
    let text = |name: &str| -> Result<String, ProjectError> {
        object
            .get(name)
            .and_then(Json::as_str)
            .map(str::to_owned)
            .ok_or(ProjectError::WrongType {
                path: format!("overrides[{index}].{name}"),
                expected: "a string",
                found: object.get(name).map(Json::kind).unwrap_or(JsonKind::Null),
            })
    };
    let Some(parsed) = SettingKey::parse(&text("key")?) else {
        return Err(ProjectError::EmptyValue {
            path: format!("overrides[{index}].key"),
        });
    };
    let value = decode_value(
        &format!("overrides[{index}].value"),
        object.get("value").unwrap_or(&Json::Null),
    )?;
    Ok(Override {
        project: text("project")?,
        target: object
            .get("target")
            .and_then(Json::as_str)
            .map(str::to_owned),
        key: parsed,
        value,
        recorded_at_millis: object
            .get("recorded_at_unix_ms")
            .and_then(Json::as_i64)
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        ActiveVerdict, ConfigurationRefusal, MachineConfiguration, Override, ResolvedConfiguration,
        SettingKey, SettingValue, Skipped, Source, credentials,
    };
    use crate::host::network::{CredentialHealth, EnvironmentStore};
    use crate::project::record::format::ProjectFile;

    struct TempDirectory(PathBuf);

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Sandbox {
        root: PathBuf,
        _guard: TempDirectory,
    }

    fn sandbox() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("rustage-configuration-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    /// Two targets, so that "which target" is a question with more than one answer.
    const PROJECT: &str = "{\"format\": 2, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
         \"pages\": [{\"document\": \"pages/main.json\"}], \
         \"targets\": [{\"name\": \"desktop\", \"profile\": \"debug\", \"features\": [\"desktop\"]}, \
                       {\"name\": \"release\", \"profile\": \"release\", \"features\": [\"desktop\", \"gpu\"]}, \
                       {\"name\": \"quiet\"}], \
         \"generated\": [{\"output\": \"src/generated/main.rs\", \"from\": \"pages/main.json\"}]}";

    /// A second project: another identity, and a target the first one does not have.
    const OTHER_PROJECT: &str = "{\"format\": 2, \"project\": {\"id\": \"9c77\", \"name\": \"Other\"}, \
         \"pages\": [{\"document\": \"pages/main.json\"}], \
         \"targets\": [{\"name\": \"handheld\", \"profile\": \"debug\"}], \
         \"generated\": []}";

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        fn project(&self, text: &str) -> ProjectFile {
            let path = self.write("project.json", text);
            ProjectFile::open(&path).expect("the project opens")
        }

        fn state(&self) -> MachineConfiguration {
            MachineConfiguration::empty(self.root.join("configuration.json"))
        }
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn every_value_says_where_it_came_from() {
        let sandbox = sandbox();
        let file = sandbox.project(PROJECT);
        let document = file.document().expect("a document");
        let machine = sandbox.state();

        let resolved =
            ResolvedConfiguration::of(document, Some("release"), &machine).expect("resolved");
        assert_eq!(resolved.target_source(), &super::TargetSource::Asked);
        let profile = resolved.setting(&SettingKey::Profile).expect("a profile");
        assert_eq!(profile.value(), &SettingValue::Word("release".to_owned()));
        assert_eq!(
            profile.source(),
            &Source::Target {
                name: "release".to_owned()
            }
        );
        assert_eq!(
            profile.replaced(),
            ["`debug` from the built-in default"],
            "the value it replaced is named with the layer that gave it"
        );
        assert!(
            profile.render().contains("from target `release`"),
            "{}",
            profile.render()
        );

        // A target that says nothing about a setting does not speak for it: the built-in default
        // decides, nothing is recorded as replaced, and the report says where the value came from
        // rather than implying the project chose it.
        let silent =
            ResolvedConfiguration::of(document, Some("quiet"), &machine).expect("resolved");
        let profile = silent.setting(&SettingKey::Profile).expect("a profile");
        assert_eq!(profile.value(), &SettingValue::Word("debug".to_owned()));
        assert_eq!(profile.source(), &Source::BuiltIn);
        assert!(profile.replaced().is_empty());
        assert!(
            profile.render().contains("from the built-in default"),
            "{}",
            profile.render()
        );
        assert_eq!(
            silent
                .setting(&SettingKey::Features)
                .expect("features")
                .value(),
            &SettingValue::List(Vec::new())
        );

        // A target that does name a value replaced the built-in one, and says so.
        let desktop = ResolvedConfiguration::of(document, None, &machine).expect("resolved");
        assert_eq!(desktop.target(), "desktop");
        assert_eq!(
            desktop
                .setting(&SettingKey::Features)
                .expect("features")
                .value(),
            &SettingValue::List(vec!["desktop".to_owned()])
        );
        assert_eq!(
            desktop
                .setting(&SettingKey::Features)
                .expect("features")
                .source(),
            &Source::Target {
                name: "desktop".to_owned()
            }
        );
    }

    #[test]
    fn a_machine_override_applies_to_its_own_project_and_says_what_it_replaced() {
        let sandbox = sandbox();
        let file = sandbox.project(PROJECT);
        let document = file.document().expect("a document");
        let mut machine = sandbox.state();
        machine.remember(
            Override::new(
                document.identity().id(),
                None,
                SettingKey::Profile,
                SettingValue::Word("release".to_owned()),
                at(1_000),
            )
            .expect("an override"),
        );

        let resolved = ResolvedConfiguration::of(document, None, &machine).expect("resolved");
        let profile = resolved.setting(&SettingKey::Profile).expect("a profile");
        assert_eq!(profile.value(), &SettingValue::Word("release".to_owned()));
        assert_eq!(
            profile.source(),
            &Source::Override {
                project: "3f2b".to_owned(),
                target: None
            }
        );
        assert_eq!(
            profile.replaced(),
            [
                "`debug` from the built-in default",
                "`debug` from target `desktop`"
            ],
            "the target named a value too, so the override replaced two layers"
        );
        assert_eq!(resolved.applied().len(), 1);
        assert!(resolved.skipped().is_empty());
        assert!(
            resolved
                .render()
                .iter()
                .any(|line| line.contains("applied override"))
        );
    }

    #[test]
    fn an_active_configuration_does_not_leak_between_projects() {
        let sandbox = sandbox();
        let first = sandbox.project(PROJECT);
        let document = first.document().expect("a document");
        let mut machine = sandbox.state();
        // A person chooses `release` for the first project …
        machine.set_active(document.identity().id(), "Demo", "release", at(1_000));

        // … and with no target asked for, that project gets it.
        let chosen = ResolvedConfiguration::of(document, None, &machine).expect("resolved");
        assert_eq!(chosen.target(), "release");

        // Another project is not given it: the choice belongs to an identity, and a project that has
        // no choice of its own decides for itself.
        let other_path = sandbox.write("other/project.json", OTHER_PROJECT);
        let other = ProjectFile::open(&other_path).expect("the other project opens");
        let other_document = other.document().expect("a document");
        assert!(matches!(
            machine.active_for(other_document.identity().id()),
            ActiveVerdict::AnotherProject(choice) if choice.target() == "release"
        ));
        let theirs = ResolvedConfiguration::of(other_document, None, &machine).expect("resolved");
        assert_eq!(theirs.target(), "handheld");
        assert_eq!(theirs.project(), "9c77");

        // Asking for the other project's target is a refusal that names what this project has, not a
        // quiet fall-back to a target nobody chose.
        let refusal = ResolvedConfiguration::of(other_document, Some("release"), &machine)
            .expect_err("no such target");
        assert_eq!(
            refusal,
            ConfigurationRefusal::NoSuchTarget {
                asked: "release".to_owned(),
                known: vec!["handheld".to_owned()]
            }
        );
        assert!(refusal.message().contains("`handheld`"));

        // A configuration carries the identity it was resolved for, and is refused for another.
        assert_eq!(
            theirs.for_project("9c77").map(|c| c.target()),
            Ok("handheld")
        );
        let mismatch = chosen.for_project("9c77").expect_err("another project");
        assert!(
            mismatch
                .message()
                .contains("is not shared between projects")
        );
        assert!(mismatch.message().contains("3f2b"));
    }

    #[test]
    fn an_override_recorded_for_another_project_or_target_is_skipped_with_the_reason() {
        let sandbox = sandbox();
        let file = sandbox.project(PROJECT);
        let document = file.document().expect("a document");
        let mut machine = sandbox.state();
        machine.remember(
            Override::new(
                "9c77",
                None,
                SettingKey::Profile,
                SettingValue::Word("release".to_owned()),
                at(1_000),
            )
            .expect("an override"),
        );
        machine.remember(
            Override::new(
                "3f2b",
                Some("release".to_owned()),
                SettingKey::Features,
                SettingValue::list(["desktop", "gpu", "raytracing"]),
                at(1_001),
            )
            .expect("an override"),
        );
        machine.remember(
            Override::new(
                "3f2b",
                None,
                SettingKey::Unrecognised("linker".to_owned()),
                SettingValue::Word("lld".to_owned()),
                at(1_002),
            )
            .expect("an override"),
        );

        // For the target the override names, it applies, and it replaces the target's own list —
        // which is reported, so a removed feature cannot look like one nobody asked for.
        let resolved = ResolvedConfiguration::of(document, Some("release"), &machine).expect("ok");
        assert_eq!(resolved.applied().len(), 1, "{:?}", resolved.applied());
        let features = resolved.setting(&SettingKey::Features).expect("features");
        assert_eq!(
            features.value(),
            &SettingValue::List(vec![
                "desktop".to_owned(),
                "gpu".to_owned(),
                "raytracing".to_owned()
            ])
        );
        assert_eq!(
            features.replaced(),
            [
                "`no features` from the built-in default",
                "`desktop, gpu` from target `release`"
            ]
        );
        let reasons: Vec<String> = resolved
            .skipped()
            .iter()
            .map(|skipped: &Skipped| skipped.reason().render())
            .collect();
        assert_eq!(reasons.len(), 2, "{reasons:?}");
        assert!(reasons[0].contains("another project"));
        assert!(
            reasons[1].contains("does not know the setting `linker`"),
            "{reasons:?}"
        );

        // For another target, nothing applies: the override belongs to `release`, the other project
        // is somebody else's, and the unknown setting stays unknown. All three are reported.
        let elsewhere = ResolvedConfiguration::of(document, Some("desktop"), &machine).expect("ok");
        assert!(elsewhere.applied().is_empty(), "{:?}", elsewhere.applied());
        let reasons: Vec<String> = elsewhere
            .skipped()
            .iter()
            .map(|skipped: &Skipped| skipped.reason().render())
            .collect();
        assert_eq!(reasons.len(), 3, "{reasons:?}");
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("another project"))
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason
                    .contains("recorded for target `release`, and `desktop` is active")),
            "{reasons:?}"
        );
        assert!(reasons.iter().any(|reason| reason.contains("`linker`")));
        assert_eq!(
            elsewhere
                .setting(&SettingKey::Features)
                .expect("features")
                .value(),
            &SettingValue::List(vec!["desktop".to_owned()])
        );
    }

    #[test]
    fn a_choice_whose_target_is_gone_is_refused_rather_than_replaced() {
        let sandbox = sandbox();
        let file = sandbox.project(OTHER_PROJECT);
        let document = file.document().expect("a document");
        let mut machine = sandbox.state();
        machine.set_active(document.identity().id(), "Other", "handheld", at(1_000));
        // The project is edited so that the target it chose no longer exists.
        let edited = sandbox.write(
            "project.json",
            &OTHER_PROJECT.replace("\"handheld\"", "\"tablet\""),
        );
        let again = ProjectFile::open(&edited).expect("the project opens");
        let refusal =
            ResolvedConfiguration::of(again.document().expect("a document"), None, &machine)
                .expect_err("a stale choice");
        assert_eq!(
            refusal,
            ConfigurationRefusal::StaleChoice {
                chosen: "handheld".to_owned(),
                known: vec!["tablet".to_owned()]
            }
        );
        assert!(
            refusal
                .message()
                .contains("does not silently build another one")
        );
    }

    #[test]
    fn a_value_whose_shape_does_not_fit_the_setting_is_refused() {
        let refusal = Override::new(
            "3f2b",
            None,
            SettingKey::Features,
            SettingValue::Word("gpu".to_owned()),
            at(1_000),
        )
        .expect_err("features are a list");
        assert_eq!(
            refusal,
            ConfigurationRefusal::WrongShape {
                key: "features".to_owned(),
                shape: "a word"
            }
        );
        assert_eq!(SettingKey::parse("profile"), Some(SettingKey::Profile));
        assert_eq!(SettingKey::parse(""), None);
        assert_eq!(
            SettingKey::parse("linker"),
            Some(SettingKey::Unrecognised("linker".to_owned()))
        );
        assert_eq!(SettingValue::list(["a", " ", "b"]).words(), ["a", "b"]);
    }

    #[test]
    fn the_state_round_trips_and_survives_a_reload() {
        let sandbox = sandbox();
        let mut machine = sandbox.state().with_capacity(2);
        machine.set_active("3f2b", "Demo", "release", at(1_000));
        machine.set_active("9c77", "Other", "handheld", at(1_001));
        machine.set_active("1234", "Third", "desktop", at(1_002));
        machine.remember(
            Override::new(
                "9c77",
                Some("release".to_owned()),
                SettingKey::Features,
                SettingValue::list(["desktop", "gpu"]),
                at(1_003),
            )
            .expect("an override"),
        );
        assert_eq!(
            machine.choices().len(),
            2,
            "the oldest choice falls off the end"
        );
        assert!(
            machine
                .choices()
                .iter()
                .all(|choice| choice.project() != "3f2b"),
            "{:?}",
            machine.choices()
        );

        let path = machine.save().expect("the state is written");
        let read = MachineConfiguration::load(&path).expect("the state is read");
        assert_eq!(read.to_json().render(), machine.to_json().render());
        assert_eq!(read.overrides().len(), 1);
        assert_eq!(read.overrides()[0].value().words(), ["desktop", "gpu"]);
        assert_eq!(read.overrides()[0].target(), Some("release"));

        // Forgetting a project takes its choice and its overrides with it, and nothing else.
        let mut read = read;
        assert_eq!(read.forget("9c77"), 2);
        assert!(
            read.choices()
                .iter()
                .all(|choice| choice.project() != "9c77")
        );
        assert!(read.overrides().is_empty());
    }

    #[test]
    fn a_state_file_from_a_newer_build_is_refused_rather_than_emptied() {
        let sandbox = sandbox();
        let path = sandbox.write(
            "configuration.json",
            "{\"format\": 2, \"active\": [], \"overrides\": []}",
        );
        let error = MachineConfiguration::load(&path).expect_err("a newer format");
        assert!(error.message().contains("newer"));

        // A file that is there and cannot be read is an error too: forgetting which target a project
        // builds because a file went wrong is a loss nobody would see.
        let broken = sandbox.write("broken.json", "{ not json");
        assert!(MachineConfiguration::load(&broken).is_err());
        // A file that is not there yet is simply empty.
        let absent = MachineConfiguration::load(sandbox.root.join("nowhere.json")).expect("empty");
        assert!(absent.choices().is_empty());
    }

    #[test]
    fn a_credential_is_shown_as_a_reference_and_its_health_never_its_material() {
        let sandbox = sandbox();
        let file = sandbox.project(
            "{\"format\": 2, \"project\": {\"id\": \"3f2b\", \"name\": \"Demo\"}, \
             \"pages\": [{\"document\": \"pages/main.json\"}], \
             \"targets\": [{\"name\": \"desktop\"}], \
             \"generated\": [], \
             \"network\": {\"services\": [{\"name\": \"registry\", \"host\": \"example.test\", \
                \"port\": 443, \"tls\": {\"roots\": \"system\"}, \
                \"credential\": {\"name\": \"registry-token\", \
                  \"source\": {\"kind\": \"environment\", \"variable\": \"RUSTAGE_TEST_TOKEN\"}}}]}}",
        );
        let document = file.document().expect("a document");
        // SAFETY: this test is the only reader of its variable, and it only asks whether it is set.
        unsafe { std::env::remove_var("RUSTAGE_TEST_TOKEN") };
        let views = credentials(document, &EnvironmentStore, at(1_000));
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].service(), "registry");
        assert_eq!(views[0].name(), "registry-token");
        assert_eq!(
            views[0].health(),
            &CredentialHealth::Missing {
                reason: "the RUSTAGE_TEST_TOKEN environment variable is not set".to_owned()
            }
        );
        let line = views[0].render();
        assert!(line.contains("registry-token"), "{line}");
        assert!(line.contains("environment variable"), "{line}");
        assert!(
            !line.contains("secret"),
            "the view says where it is, never what is in it: {line}"
        );
    }
}
