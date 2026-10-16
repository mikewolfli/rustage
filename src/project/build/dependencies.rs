// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Dependencies, as cargo reports them, and what a change would mean (task T02.11).
//!
//! Reading a dependency tree means starting a program — `cargo metadata` — and that program can do
//! more than read: without `--locked` it may **write `Cargo.lock`**, and without `--offline` it may
//! **reach the registry**. The acceptance is that the resolving command is authorized by what it
//! actually does and that nothing upgrades a dependency or touches the network implicitly, so the
//! two are one thing here: [`Resolution`] is the command *and* the effects it would have, and a
//! caller has to authorize every effect before anything runs. The default resolution carries
//! `--locked --offline`, which is read-only by construction; asking for a resolution that could
//! change something means saying so, and the effects are listed before they happen.
//!
//! What is kept from a resolution is a [`Digest`] — the packages, which of them are workspace
//! members, the features those members have enabled, and the state of the lockfile — because that is
//! what a *review* needs and it is far smaller than what cargo printed. Two digests compared
//! ([`Review::between`]) name what was added, removed, re-versioned or added a feature, which is how
//! "a dependency change" is confirmed: by looking at what changed rather than by trusting that it
//! was small.
//!
//! The lockfile is read as a lockfile and not as TOML: [`Lockfile::read`] takes the version cargo
//! wrote into it and counts its `[[package]]` sections, and says it could not read it when it cannot.
//! A hand-written TOML parser would be a second parser to drift, and a dependency tree is a bad place
//! to guess.

use std::{
    cmp::Ordering,
    fmt,
    path::{Path, PathBuf},
};

use crate::contracts::permission::EffectClass;
use crate::contracts::revision::FormatVersion;
use crate::project::record::format::ProjectError;
use crate::project::record::json::{self, Json, JsonObject};

/// The manifest a directory has to hold to be resolved.
pub const MANIFEST_FILE_NAME: &str = "Cargo.toml";

/// The lockfile a resolution may write.
pub const LOCKFILE_FILE_NAME: &str = "Cargo.lock";

/// The version of a dependency record this build writes.
pub const RECORD_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The members a dependency record has.
pub const RECORD_MEMBERS: [&str; 4] = ["format", "root", "lockfile", "packages"];

/// The members one recorded package has.
pub const PACKAGE_MEMBERS: [&str; 5] = ["id", "name", "version", "source", "features"];

/// One thing running a resolving command could do.
///
/// They are effects and not flags because that is what a caller authorizes: `--locked` and
/// `--offline` are how this build writes them down, and the mapping is stated in one place
/// ([`Resolution::read_only`] and [`Resolution::may_change_things`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Starting a program, which is what resolution always is.
    RunsAProgram,
    /// It may write the lockfile.
    MayWriteTheLockfile,
    /// It may reach the registry.
    MayReachTheRegistry,
}

impl Effect {
    /// Every effect, in the order a report lists them.
    pub const ALL: [Self; 3] = [
        Self::RunsAProgram,
        Self::MayWriteTheLockfile,
        Self::MayReachTheRegistry,
    ];

    /// The name a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RunsAProgram => "run-command",
            Self::MayWriteTheLockfile => "write-file (Cargo.lock)",
            Self::MayReachTheRegistry => "network (the registry)",
        }
    }

    /// What the host's permission table calls the same effect.
    pub const fn class(self) -> EffectClass {
        match self {
            Self::RunsAProgram => EffectClass::Command,
            Self::MayWriteTheLockfile => EffectClass::FileWrite,
            Self::MayReachTheRegistry => EffectClass::Network,
        }
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// The command that resolves a directory, and what running it would do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Resolution {
    directory: PathBuf,
    arguments: Vec<String>,
    effects: Vec<Effect>,
}

impl Resolution {
    /// The resolution this build runs when nobody asked for anything that changes.
    ///
    /// `--locked` and `--offline` are not conveniences: they are what makes the command read-only,
    /// because cargo is allowed to rewrite the lockfile when it is not locked and to fetch when it is
    /// not offline.
    pub fn read_only(directory: impl Into<PathBuf>) -> Self {
        Self::with_effects(directory, false, false)
    }

    /// The resolution that may write the lockfile, which a caller has to ask for and authorize.
    ///
    /// It is still offline: cargo resolves from what this machine already has and records the
    /// versions it chose, which is a change to a file and not a trip to the registry.
    pub fn may_write_the_lockfile(directory: impl Into<PathBuf>) -> Self {
        Self::with_effects(directory, true, false)
    }

    /// The resolution that may reach the registry, which a caller has to ask for and authorize.
    pub fn may_reach_the_registry(directory: impl Into<PathBuf>) -> Self {
        Self::with_effects(directory, false, true)
    }

    /// The resolution for the effects a caller asked for.
    ///
    /// The flags are the effects: `--locked` is dropped only when the caller asked for the lockfile to
    /// be writable, and `--offline` only when they asked to reach the registry. There is deliberately
    /// no way to ask for a resolution that *upgrades*: without `--locked` cargo may choose newer
    /// versions, and this build offers that only as "the lockfile may change", with the change shown
    /// afterwards.
    pub fn with_effects(
        directory: impl Into<PathBuf>,
        may_write_lockfile: bool,
        may_reach_registry: bool,
    ) -> Self {
        let directory = directory.into();
        // The manifest is named, not inherited: without it cargo would resolve whatever happens to be
        // in this process's directory, and the command that runs would not be the command reported.
        let mut arguments = vec![
            "metadata".to_owned(),
            "--manifest-path".to_owned(),
            directory.join(MANIFEST_FILE_NAME).display().to_string(),
            "--format-version".to_owned(),
            "1".to_owned(),
        ];
        if !may_write_lockfile {
            arguments.push("--locked".to_owned());
        }
        if !may_reach_registry {
            arguments.push("--offline".to_owned());
        }
        let mut effects = vec![Effect::RunsAProgram];
        if may_write_lockfile {
            effects.push(Effect::MayWriteTheLockfile);
        }
        if may_reach_registry {
            effects.push(Effect::MayReachTheRegistry);
        }
        Self {
            directory,
            arguments,
            effects,
        }
    }

    /// The directory it resolves.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The arguments, after the program.
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// The whole command, as a report writes it.
    pub fn command(&self) -> String {
        format!("cargo {}", self.arguments.join(" "))
    }

    /// What running it could do.
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// Whether it is read-only, which is what the default asks for.
    pub fn is_read_only(&self) -> bool {
        self.effects == [Effect::RunsAProgram]
    }

    /// The effects that are not authorized.
    ///
    /// A caller authorizes by *class*, which is what the host's permission table is keyed by, and
    /// this is where "the command that was asked for" meets "the effects that were agreed to".
    pub fn unauthorized(&self, authorized: &[EffectClass]) -> Vec<Effect> {
        self.effects
            .iter()
            .copied()
            .filter(|effect| !authorized.contains(&effect.class()))
            .collect()
    }

    /// One line: the command, and what it could do.
    pub fn render(&self) -> String {
        format!(
            "cargo metadata in {}: {} — {}",
            self.directory.display(),
            self.command(),
            self.effects
                .iter()
                .map(|effect| effect.name())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// What a lockfile says about itself.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Lockfile {
    /// It is there, with the version it declares and how many packages it holds.
    Present {
        /// The `version` its first section declares.
        version: u64,
        /// How many `[[package]]` sections it has.
        packages: usize,
        /// How many bytes it is.
        bytes: usize,
    },
    /// There is none, which means a resolution would be free to choose.
    Absent,
    /// There is one and it could not be read.
    Unreadable {
        /// What the operating system said.
        reason: String,
    },
}

impl Lockfile {
    /// Reads a lockfile beside a manifest.
    ///
    /// It reads two things and says so: the `version` line cargo wrote, and how many package sections
    /// there are. Parsing TOML properly is not what this is for, and a value it cannot find is
    /// reported as missing rather than guessed.
    pub fn read(directory: &Path) -> Self {
        let path = directory.join(LOCKFILE_FILE_NAME);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Self::Absent,
            Err(error) => {
                return Self::Unreadable {
                    reason: error.to_string(),
                };
            }
        };
        let version = text
            .lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix("version"))
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .and_then(|rest| rest.trim().trim_matches('"').parse::<u64>().ok())
            .unwrap_or_default();
        let packages = text
            .lines()
            .filter(|line| line.trim() == "[[package]]")
            .count();
        Self::Present {
            version,
            packages,
            bytes: text.len(),
        }
    }

    /// How many packages it holds, when it is there.
    pub const fn packages(&self) -> Option<usize> {
        match self {
            Self::Present { packages, .. } => Some(*packages),
            _ => None,
        }
    }

    /// Whether anything can be resolved at all: a lockfile that is not there means every version is
    /// still a choice.
    pub const fn is_present(&self) -> bool {
        matches!(self, Self::Present { .. })
    }

    /// One line for a report.
    pub fn render(&self) -> String {
        match self {
            Self::Present {
                version,
                packages,
                bytes,
            } => format!("{LOCKFILE_FILE_NAME} v{version}, {packages} package(s), {bytes} byte(s)"),
            Self::Absent => format!(
                "no {LOCKFILE_FILE_NAME}: every version is still a choice, and a locked resolution \
                 would refuse"
            ),
            Self::Unreadable { reason } => {
                format!("{LOCKFILE_FILE_NAME} could not be read: {reason}")
            }
        }
    }

    fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        match self {
            Self::Present {
                version,
                packages,
                bytes,
            } => {
                out.insert("state", Json::string("present"));
                out.insert("version", Json::number(*version));
                out.insert("packages", Json::number(*packages as u64));
                out.insert("bytes", Json::number(*bytes as u64));
            }
            Self::Absent => out.insert("state", Json::string("absent")),
            Self::Unreadable { reason } => {
                out.insert("state", Json::string("unreadable"));
                out.insert("reason", Json::string(reason));
            }
        }
        Json::Object(out)
    }

    fn from_json(value: &Json, at: &str) -> Result<Self, ProjectError> {
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: format!("`{at}`"),
                expected: "an object",
                found: value.kind(),
            });
        };
        let state = object.get("state").and_then(Json::as_str).ok_or_else(|| {
            ProjectError::MissingMember {
                path: format!("`{at}.state`"),
            }
        })?;
        match state {
            "present" => Ok(Self::Present {
                version: number_or_zero(object, "version"),
                packages: number_or_zero(object, "packages") as usize,
                bytes: number_or_zero(object, "bytes") as usize,
            }),
            "absent" => Ok(Self::Absent),
            "unreadable" => Ok(Self::Unreadable {
                reason: object
                    .get("reason")
                    .and_then(Json::as_str)
                    .unwrap_or("the record does not say why")
                    .to_owned(),
            }),
            _ => Err(ProjectError::Inconsistent {
                path: format!("`{at}.state`"),
                reason: format!("`{state}` is not one of present, absent or unreadable"),
            }),
        }
    }
}

fn number_or_zero(object: &JsonObject, member: &str) -> u64 {
    object
        .get(member)
        .and_then(Json::as_u64)
        .unwrap_or_default()
}

/// One package in a resolution.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Package {
    id: String,
    name: String,
    version: String,
    source: String,
    member: bool,
    features: Vec<String>,
}

impl Package {
    /// Cargo's own identifier, which is what makes two packages comparable across resolutions.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The package's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The version that was resolved.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Where it comes from: a registry, a path, or nothing for a workspace member.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether it is a workspace member, which is the difference between the project's own code and
    /// what it depends on.
    pub fn is_member(&self) -> bool {
        self.member
    }

    /// The features enabled for it, which is part of what a change can be.
    pub fn features(&self) -> &[String] {
        &self.features
    }

    fn render(&self) -> String {
        // The version column is padded so the sources line up, and the result is trimmed because a
        // package with no source and no features would otherwise end the line in spaces.
        format!(
            "{:<32} {:<12} {}{}{}",
            self.id,
            self.version,
            self.source,
            if self.member { "  [member]" } else { "" },
            if self.features.is_empty() {
                String::new()
            } else {
                format!("  features: {}", self.features.join(", "))
            }
        )
        .trim_end()
        .to_owned()
    }
}

/// What one resolution said, kept small enough to record and compare.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Digest {
    root: String,
    metadata_version: u64,
    lockfile: Lockfile,
    members: Vec<String>,
    packages: Vec<Package>,
}

impl Digest {
    /// Reads cargo's own report.
    ///
    /// This is a reader for what `cargo metadata --format-version 1` prints and nothing else: the
    /// packages, which of them are workspace members, and the features each resolve node has enabled.
    /// A report that does not have those members is refused rather than read as a small project.
    pub fn of_cargo_metadata(text: &str, named: &Path) -> Result<Self, ProjectError> {
        let value = json::parse(text).map_err(|error| ProjectError::NotJson {
            path: PathBuf::from("cargo metadata"),
            error,
        })?;
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "cargo metadata".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        let metadata_version = object
            .get("version")
            .and_then(Json::as_u64)
            .unwrap_or_default();
        let root = object
            .get("workspace_root")
            .and_then(Json::as_str)
            .ok_or_else(|| ProjectError::MissingMember {
                path: "`workspace_root` in cargo metadata".to_owned(),
            })?
            .to_owned();
        let Some(members) = object.get("workspace_members").and_then(Json::as_array) else {
            return Err(ProjectError::MissingMember {
                path: "`workspace_members` in cargo metadata".to_owned(),
            });
        };
        let member_ids: Vec<String> = members
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect();
        let Some(packages) = object.get("packages").and_then(Json::as_array) else {
            return Err(ProjectError::MissingMember {
                path: "`packages` in cargo metadata".to_owned(),
            });
        };
        // The resolve section is where the enabled features are, and a package that is in it has them;
        // a package that is not in it (a build dependency of a target nobody asked about, say) is
        // reported with none rather than with an invented set.
        let features_of = |id: &str| -> Vec<String> {
            object
                .get("resolve")
                .and_then(Json::as_object)
                .and_then(|resolve| resolve.get("nodes"))
                .and_then(Json::as_array)
                .and_then(|nodes| {
                    nodes.iter().find(|node| {
                        node.as_object()
                            .and_then(|node| node.get("id"))
                            .and_then(Json::as_str)
                            == Some(id)
                    })
                })
                .and_then(Json::as_object)
                .and_then(|node| node.get("features"))
                .and_then(Json::as_array)
                .map(|features| {
                    features
                        .iter()
                        .filter_map(Json::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };

        let mut out = Vec::new();
        for (index, entry) in packages.iter().enumerate() {
            let at = format!("packages[{index}]");
            let Some(package) = entry.as_object() else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}`"),
                    expected: "an object",
                    found: entry.kind(),
                });
            };
            let id = text_of(package, "id", &at)?;
            let name = text_of(package, "name", &at)?;
            let version = text_of(package, "version", &at)?;
            let source = package
                .get("source")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            let member = member_ids.iter().any(|there| there == &id);
            let features = features_of(&id);
            out.push(Package {
                id,
                name,
                version,
                source,
                member,
                features,
            });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));

        // The lockfile is read where cargo keeps it, which the report itself says: "the lockfile
        // beside the directory that was named" is a guess that is wrong for every member of a
        // workspace, and a report that named the wrong lockfile would be a report that lies.
        let lockfile = Lockfile::read(&lockfile_beside(named, &root));

        Ok(Self {
            root,
            metadata_version,
            lockfile,
            members: member_ids,
            packages: out,
        })
    }

    /// The workspace root cargo reported.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The format version of the report this came from.
    pub const fn metadata_version(&self) -> u64 {
        self.metadata_version
    }

    /// What the lockfile says.
    pub const fn lockfile(&self) -> &Lockfile {
        &self.lockfile
    }

    /// Every package, ordered by cargo's own identifier.
    pub fn packages(&self) -> &[Package] {
        &self.packages
    }

    /// The packages that are workspace members.
    pub fn workspace_members(&self) -> Vec<&Package> {
        self.packages
            .iter()
            .filter(|package| package.member)
            .collect()
    }

    /// The packages that are dependencies rather than the project's own code.
    pub fn dependencies(&self) -> Vec<&Package> {
        self.packages
            .iter()
            .filter(|package| !package.member)
            .collect()
    }

    /// One line per package, with the members first.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "cargo metadata v{} for {}, {} package(s), {} of them workspace members",
                self.metadata_version,
                self.root,
                self.packages.len(),
                self.workspace_members().len()
            ),
            format!("  {}", self.lockfile.render()),
        ];
        for package in self.workspace_members() {
            lines.push(format!("  member {}", package.render()));
        }
        for package in self.dependencies() {
            lines.push(format!("  dep    {}", package.render()));
        }
        lines
    }

    /// The record as JSON.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(RECORD_FORMAT.get())));
        out.insert("root", Json::string(&self.root));
        let mut metadata = JsonObject::new();
        metadata.insert("version", Json::number(self.metadata_version));
        metadata.insert("lockfile", self.lockfile.to_json());
        metadata.insert(
            "members",
            Json::array(self.members.iter().map(Json::string).collect::<Vec<_>>()),
        );
        out.insert("metadata", Json::Object(metadata));
        out.insert(
            "packages",
            Json::array(
                self.packages
                    .iter()
                    .map(|package| {
                        let mut entry = JsonObject::new();
                        entry.insert("id", Json::string(&package.id));
                        entry.insert("name", Json::string(&package.name));
                        entry.insert("version", Json::string(&package.version));
                        entry.insert("source", Json::string(&package.source));
                        entry.insert(
                            "features",
                            Json::array(
                                package
                                    .features
                                    .iter()
                                    .map(Json::string)
                                    .collect::<Vec<_>>(),
                            ),
                        );
                        entry.insert("member", Json::bool(package.member));
                        Json::Object(entry)
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        Json::Object(out)
    }

    /// Writes the record, through a temporary file and a rename.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<PathBuf, ProjectError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| ProjectError::Io {
                path: parent.to_path_buf(),
                action: "creating the directory the dependency record goes in",
                reason: error.to_string(),
            })?;
        }
        json::write_atomically(&path, &self.to_json().render()).map_err(|error| {
            ProjectError::Io {
                path: path.clone(),
                action: "writing the dependency record",
                reason: error.to_string(),
            }
        })?;
        Ok(path)
    }

    /// Reads a record written by [`Digest::save`].
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading the dependency record to compare against",
            reason: error.to_string(),
        })?;
        let value = json::parse(&text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let version = crate::project::record::format::format_version(&value)?;
        match version.get().cmp(&RECORD_FORMAT.get()) {
            Ordering::Equal => {}
            Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: RECORD_FORMAT,
                });
            }
            Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: RECORD_FORMAT,
                });
            }
        }
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "the dependency record".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        let root = object
            .get("root")
            .and_then(Json::as_str)
            .ok_or_else(|| ProjectError::MissingMember {
                path: "`root` in the dependency record".to_owned(),
            })?
            .to_owned();
        let metadata = object
            .get("metadata")
            .and_then(Json::as_object)
            .ok_or_else(|| ProjectError::MissingMember {
                path: "`metadata` in the dependency record".to_owned(),
            })?;
        let lockfile = Lockfile::from_json(
            metadata
                .get("lockfile")
                .ok_or_else(|| ProjectError::MissingMember {
                    path: "`metadata.lockfile`".to_owned(),
                })?,
            "metadata.lockfile",
        )?;
        let members = metadata
            .get("members")
            .and_then(Json::as_array)
            .map(|members| {
                members
                    .iter()
                    .filter_map(Json::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let Some(packages) = object.get("packages").and_then(Json::as_array) else {
            return Err(ProjectError::MissingMember {
                path: "`packages` in the dependency record".to_owned(),
            });
        };
        let mut out = Vec::new();
        for (index, entry) in packages.iter().enumerate() {
            let at = format!("packages[{index}]");
            let Some(package) = entry.as_object() else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}`"),
                    expected: "an object",
                    found: entry.kind(),
                });
            };
            out.push(Package {
                id: text_of(package, "id", &at)?,
                name: text_of(package, "name", &at)?,
                version: text_of(package, "version", &at)?,
                source: package
                    .get("source")
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                member: package
                    .get("member")
                    .and_then(Json::as_bool)
                    .unwrap_or(false),
                features: package
                    .get("features")
                    .and_then(Json::as_array)
                    .map(|features| {
                        features
                            .iter()
                            .filter_map(Json::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
        Ok(Self {
            root,
            metadata_version: metadata
                .get("version")
                .and_then(Json::as_u64)
                .unwrap_or_default(),
            lockfile,
            members,
            packages: out,
        })
    }

    /// Whether two digests are of the same workspace, which is what makes them comparable.
    pub fn is_the_same_workspace_as(&self, other: &Digest) -> bool {
        self.root == other.root
    }
}

fn text_of(object: &JsonObject, member: &str, at: &str) -> Result<String, ProjectError> {
    object
        .get(member)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProjectError::MissingMember {
            path: format!("`{at}.{member}`"),
        })
}

/// How one package changed between two resolutions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    /// It is in the newer resolution and was not in the older one.
    Added,
    /// It was in the older one and is not in the newer one.
    Removed,
    /// It is in both, at a different version.
    Version,
    /// It is in both at the same version, and the features enabled for it are not the same.
    Features,
}

impl ChangeKind {
    /// The name a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Version => "version",
            Self::Features => "features",
        }
    }
}

impl fmt::Display for ChangeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// One package that is not the same in two resolutions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    kind: ChangeKind,
    package: String,
    name: String,
    from: Option<String>,
    to: Option<String>,
}

impl Change {
    /// How it changed.
    pub const fn kind(&self) -> ChangeKind {
        self.kind
    }

    /// Cargo's identifier, which is what makes it the same package across two resolutions.
    pub fn package(&self) -> &str {
        &self.package
    }

    /// The package's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it was, when it was there before.
    pub fn from(&self) -> Option<&str> {
        self.from.as_deref()
    }

    /// What it is now, when it is there now.
    pub fn to(&self) -> Option<&str> {
        self.to.as_deref()
    }

    /// Whether it is the project's own code rather than a dependency.
    pub fn is_a_member(&self) -> bool {
        self.package.contains("path+file://")
    }

    /// One line.
    pub fn render(&self) -> String {
        match (self.from.as_deref(), self.to.as_deref()) {
            (None, Some(to)) => format!("{:<8} {:<40} {to}", self.kind.name(), self.name),
            (Some(from), None) => format!("{:<8} {:<40} was {from}", self.kind.name(), self.name),
            (Some(from), Some(to)) => {
                format!("{:<8} {:<40} {from} → {to}", self.kind.name(), self.name)
            }
            (None, None) => format!("{:<8} {}", self.kind.name(), self.name),
        }
    }
}

/// What changed between two resolutions.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Review {
    root: String,
    changes: Vec<Change>,
    unchanged: usize,
    lockfile: (String, String),
    metadata: (u64, u64),
}

impl Review {
    /// Compares two digests.
    ///
    /// A resolution of a *different* workspace is refused rather than compared: two workspaces can
    /// each have a `serde`, and reporting one as a change to the other is exactly the kind of
    /// cross-project confusion a record is keyed by its root to avoid.
    pub fn between(before: &Digest, after: &Digest) -> Result<Self, ReviewRefusal> {
        if !before.is_the_same_workspace_as(after) {
            return Err(ReviewRefusal::AnotherWorkspace {
                expected: before.root.clone(),
                found: after.root.clone(),
            });
        }
        let mut changes = Vec::new();
        let mut unchanged = 0usize;
        let mut removed: Vec<Package> = Vec::new();
        let mut seen = vec![false; after.packages.len()];
        for old in &before.packages {
            match after.packages.iter().position(|new| new.id == old.id) {
                Some(index) => {
                    seen[index] = true;
                    let new = &after.packages[index];
                    // Cargo's identifier is version-qualified, so the same identifier means the same
                    // version; a report where it does not is still read, because silently calling it
                    // unchanged would hide exactly what a review is for.
                    if new.version != old.version {
                        changes.push(Change {
                            kind: ChangeKind::Version,
                            package: old.id.clone(),
                            name: old.name.clone(),
                            from: Some(old.version.clone()),
                            to: Some(new.version.clone()),
                        });
                    }
                    if new.features != old.features {
                        changes.push(Change {
                            kind: ChangeKind::Features,
                            package: old.id.clone(),
                            name: old.name.clone(),
                            from: Some(summarise(&old.features)),
                            to: Some(summarise(&new.features)),
                        });
                    }
                    if new.version == old.version && new.features == old.features {
                        unchanged += 1;
                    }
                }
                None => removed.push(old.clone()),
            }
        }
        let mut added: Vec<Package> = after
            .packages
            .iter()
            .enumerate()
            .filter(|(index, _)| !seen[*index])
            .map(|(_, package)| package.clone())
            .collect();

        // Cargo's identifier holds the version, so a package that was re-versioned is *two*
        // identifiers rather than one that changed. Pairing a removal with an addition that has the
        // same name and source is what turns that into "serde 1.0.200 → 1.0.210"; a crate that has
        // two versions at once keeps the second one as an addition, which is what it is.
        for old in removed {
            match added
                .iter()
                .position(|new| new.name == old.name && new.source == old.source)
            {
                Some(index) => {
                    let new = added.remove(index);
                    changes.push(Change {
                        kind: ChangeKind::Version,
                        package: new.id.clone(),
                        name: new.name.clone(),
                        from: Some(old.version),
                        to: Some(new.version),
                    });
                }
                None => changes.push(Change {
                    kind: ChangeKind::Removed,
                    package: old.id.clone(),
                    name: old.name.clone(),
                    from: Some(old.version),
                    to: None,
                }),
            }
        }
        for new in added {
            changes.push(Change {
                kind: ChangeKind::Added,
                package: new.id.clone(),
                name: new.name.clone(),
                from: None,
                to: Some(new.version),
            });
        }
        Ok(Self {
            root: after.root.clone(),
            changes,
            unchanged,
            lockfile: (before.lockfile.render(), after.lockfile.render()),
            metadata: (before.metadata_version, after.metadata_version),
        })
    }

    /// The workspace the comparison is about.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// What changed, added first and removed last.
    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    /// How many packages are exactly as they were.
    pub const fn unchanged(&self) -> usize {
        self.unchanged
    }

    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.lockfile.0 == self.lockfile.1
    }

    /// Whether the lockfile is not the same file state as before.
    pub fn lockfile_changed(&self) -> bool {
        self.lockfile.0 != self.lockfile.1
    }

    /// One line per change, then how much did not change.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.metadata.0 != self.metadata.1 {
            lines.push(format!(
                "the report format changed: it was v{} when recorded and v{} now",
                self.metadata.0, self.metadata.1
            ));
        }
        if self.lockfile_changed() {
            lines.push(format!(
                "lockfile: {} → {}",
                self.lockfile.0, self.lockfile.1
            ));
        }
        if self.changes.is_empty() {
            lines.push("since the record, no package changed".to_owned());
        } else {
            lines.push(format!(
                "since the record, {} package change(s):",
                self.changes.len()
            ));
            for change in &self.changes {
                lines.push(format!("  {}", change.render()));
            }
        }
        lines.push(format!("{} package(s) are as they were", self.unchanged));
        lines
    }
}

/// Where the lockfile of a resolution is, given the directory that was named and the root cargo
/// reported.
///
/// Cargo keeps one lockfile per workspace, at the workspace root. A member's manifest is inside a
/// workspace whose lockfile is at the root, so the two are not the same thing, and this is the rule
/// rather than the assumption: the reported root when cargo answered with one that is a directory,
/// and the directory that was named otherwise.
pub fn lockfile_beside(named: &Path, workspace_root: &str) -> PathBuf {
    let reported = Path::new(workspace_root);
    if reported.is_dir() {
        reported.to_path_buf()
    } else {
        named.to_path_buf()
    }
}

fn summarise(features: &[String]) -> String {
    match features {
        [] => "none".to_owned(),
        [one] => one.clone(),
        many => format!("{} features", many.len()),
    }
}

/// Why two resolutions could not be compared.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReviewRefusal {
    /// The record is of another workspace.
    AnotherWorkspace {
        /// The workspace the record is of.
        expected: String,
        /// The workspace being looked at.
        found: String,
    },
}

impl ReviewRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::AnotherWorkspace { expected, found } => format!(
                "the record is of {expected}, and this is {found}; a dependency record belongs to \
                 one workspace, so nothing was compared"
            ),
        }
    }
}

impl fmt::Display for ReviewRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for ReviewRefusal {}

/// Why a directory cannot be resolved at all.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DependencyRefusal {
    /// There is no manifest, so there is nothing to resolve.
    NoManifest {
        /// Where it would be.
        path: PathBuf,
    },
    /// A record may not be written over the manifest a workspace is resolved from.
    TheManifest {
        /// The path.
        path: PathBuf,
    },
    /// A record may not be written over the lockfile, which is what a resolution reads and may write.
    TheLockfile {
        /// The path.
        path: PathBuf,
    },
}

/// Whether a path can hold a dependency record for a workspace.
///
/// A record is this host's artifact about a workspace, so it is not part of it: writing it over the
/// manifest or the lockfile would destroy the thing it describes, and both are one `canonicalize`
/// away from looking like an ordinary path.
pub fn may_record_at(directory: &Path, path: &Path) -> Result<(), DependencyRefusal> {
    let same = |other: PathBuf| -> bool {
        let canonical =
            |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        canonical(path) == canonical(&other)
    };
    if same(directory.join(MANIFEST_FILE_NAME)) {
        return Err(DependencyRefusal::TheManifest {
            path: path.to_path_buf(),
        });
    }
    if same(directory.join(LOCKFILE_FILE_NAME)) {
        return Err(DependencyRefusal::TheLockfile {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

impl DependencyRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::NoManifest { path } => format!(
                "there is no {MANIFEST_FILE_NAME} at {}: a dependency tree is what cargo resolves \
                 from a manifest",
                path.display()
            ),
            Self::TheManifest { path } => format!(
                "{} is the manifest itself; a dependency record goes to its own file",
                path.display()
            ),
            Self::TheLockfile { path } => format!(
                "{} is the lockfile; a dependency record goes to its own file",
                path.display()
            ),
        }
    }
}

impl fmt::Display for DependencyRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for DependencyRefusal {}

/// The manifest a resolution needs, or a refusal.
pub fn manifest_of(directory: &Path) -> Result<PathBuf, DependencyRefusal> {
    let manifest = directory.join(MANIFEST_FILE_NAME);
    if manifest.is_file() {
        Ok(manifest)
    } else {
        Err(DependencyRefusal::NoManifest { path: manifest })
    }
}

/// The reason a resolution failed, taken from what it printed.
///
/// A resolution can fail for a reason that is itself the finding — a lockfile that does not match the
/// manifest is exactly "a dependency change is pending" — so the reason is kept rather than replaced
/// by a generic failure. It is only asked about a run that ended badly: a successful run that merely
/// mentions a flag is not a failure, and this does not pretend to tell the two apart on its own.
pub fn failure_reason(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("error"))
        .map(str::to_owned)
}

/// Whether a failure is about the lockfile not matching the manifest.
///
/// It is asked about a reason [`failure_reason`] already took, so the answer is about the failure and
/// not about whether the command mentioned a flag.
pub fn is_a_lockfile_disagreement(reason: &str) -> bool {
    reason.contains("--locked") || reason.contains("lock file") || reason.contains("Cargo.lock")
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::{
        Digest, Effect, LOCKFILE_FILE_NAME, Lockfile, RECORD_MEMBERS, Resolution, Review,
        ReviewRefusal, manifest_of,
    };
    use crate::contracts::permission::EffectClass;

    /// A directory that is not on this machine, so a fixture report has no lockfile unless the test
    /// writes one: `Lockfile::read` of a path that is not there is `Absent`, which is the truth.
    const NOWHERE: &str = "/nonexistent/rustage-fixture-workspace";

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
        let root = std::env::temp_dir().join(format!("rustage-deps-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary directory is creatable");
        Sandbox {
            root: root.clone(),
            _guard: TempDirectory(root),
        }
    }

    impl Sandbox {
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a directory");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }
    }

    /// What `cargo metadata --format-version 1` prints, cut down to what a digest keeps.
    fn metadata(packages: &str, members: &str, resolved: &str) -> String {
        metadata_at(Path::new(NOWHERE), packages, members, resolved)
    }

    fn metadata_at(root: &Path, packages: &str, members: &str, resolved: &str) -> String {
        format!(
            "{{\"packages\": [{packages}], \"workspace_members\": [{members}], \
             \"workspace_default_members\": [{members}], \
             \"resolve\": {{\"nodes\": [{resolved}], \"root\": null}}, \
             \"target_directory\": \"/tmp/target\", \"version\": 1, \
             \"workspace_root\": \"{}\", \"metadata\": null}}",
            root.display()
        )
    }

    fn package(id: &str, name: &str, version: &str, source: &str) -> String {
        format!(
            "{{\"name\": \"{name}\", \"version\": \"{version}\", \"id\": \"{id}\", \
             \"license\": null, \"license_file\": null, \"description\": null, \
             \"source\": {source}, \"dependencies\": [], \"targets\": [], \"features\": {{}}, \
             \"manifest_path\": \"/tmp/area/{name}/Cargo.toml\", \"metadata\": null, \
             \"publish\": null, \"authors\": [], \"categories\": [], \"keywords\": [], \
             \"readme\": null, \"repository\": null, \"edition\": \"2024\", \"links\": null, \
             \"default_run\": null, \"rust_version\": null}}"
        )
    }

    /// A registry package the way cargo names one: the version is part of the identifier.
    fn registry(name: &str, version: &str) -> String {
        package(
            &format!("registry+https://github.com/rust-lang/crates.io-index#{name}@{version}"),
            name,
            version,
            "\"registry+https://github.com/rust-lang/crates.io-index\"",
        )
    }

    const MEMBER_ID: &str = "path+file:///tmp/area/app#app@0.1.0";
    const SERDE_ID: &str = "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.200";

    fn one_member() -> String {
        one_member_at(Path::new(NOWHERE))
    }

    fn one_member_at(root: &Path) -> String {
        metadata_at(
            root,
            &format!(
                "{}, {}, {}",
                package(MEMBER_ID, "app", "0.1.0", "null"),
                package(
                    SERDE_ID,
                    "serde",
                    "1.0.200",
                    "\"registry+https://github.com/rust-lang/crates.io-index\""
                ),
                registry("libc", "0.2.155")
            ),
            &format!("\"{MEMBER_ID}\""),
            &format!(
                "{{\"id\": \"{MEMBER_ID}\", \"dependencies\": [\"{SERDE_ID}\"], \
                  \"deps\": [], \"features\": [\"default\"]}}"
            ),
        )
    }

    #[test]
    fn the_command_a_resolution_would_run_says_what_it_would_do() {
        let read_only = Resolution::read_only("/tmp/area");
        assert_eq!(
            read_only.command(),
            "cargo metadata --manifest-path /tmp/area/Cargo.toml --format-version 1 --locked --offline"
        );
        assert!(read_only.is_read_only());
        assert_eq!(read_only.effects(), [Effect::RunsAProgram]);
        assert_eq!(read_only.unauthorized(&[EffectClass::Command]), Vec::new());
        assert_eq!(
            read_only.unauthorized(&[]),
            vec![Effect::RunsAProgram],
            "a command nobody authorized is not run"
        );

        // Asking for one effect drops exactly the flag that holds it:
        let writable = Resolution::may_write_the_lockfile("/tmp/area");
        assert_eq!(
            writable.command(),
            "cargo metadata --manifest-path /tmp/area/Cargo.toml --format-version 1 --offline"
        );
        assert_eq!(
            writable.effects(),
            [Effect::RunsAProgram, Effect::MayWriteTheLockfile]
        );
        assert_eq!(
            writable.unauthorized(&[EffectClass::Command]),
            vec![Effect::MayWriteTheLockfile]
        );
        assert!(!writable.command().contains("--locked"));
        assert!(writable.command().contains("--offline"));
        let both = Resolution::with_effects("/tmp/area", true, true);
        assert_eq!(
            both.command(),
            "cargo metadata --manifest-path /tmp/area/Cargo.toml --format-version 1"
        );
        assert_eq!(both.effects().len(), 3);
        assert_eq!(
            both.unauthorized(&[EffectClass::Command, EffectClass::FileWrite]),
            vec![Effect::MayReachTheRegistry]
        );

        // The resolution that may reach the registry keeps `--locked`: a resolution that upgrades
        // would be the implicit upgrade the acceptance forbids, and it is not offered at all.
        let reaching = Resolution::may_reach_the_registry("/tmp/area");
        assert_eq!(
            reaching.command(),
            "cargo metadata --manifest-path /tmp/area/Cargo.toml --format-version 1 --locked"
        );
        assert!(!reaching.is_read_only());
        assert!(reaching.command().contains("--locked"));
        assert!(!reaching.command().contains("--offline"));
        assert_eq!(
            reaching.unauthorized(&[EffectClass::Command]),
            vec![Effect::MayReachTheRegistry]
        );
        assert_eq!(
            reaching.unauthorized(&[EffectClass::Command, EffectClass::Network]),
            Vec::new()
        );
        // Every effect names the permission table's own word for it, so a report and the gate agree.
        assert_eq!(Effect::RunsAProgram.class(), EffectClass::Command);
        assert_eq!(Effect::MayWriteTheLockfile.class(), EffectClass::FileWrite);
        assert_eq!(Effect::MayReachTheRegistry.class(), EffectClass::Network);
        assert!(Effect::ALL.iter().all(|effect| !effect.name().is_empty()));
        assert!(read_only.render().contains("run-command"));
    }

    #[test]
    fn a_directory_without_a_manifest_is_not_resolved() {
        let sandbox = sandbox();
        let refusal = manifest_of(&sandbox.root).expect_err("there is no Cargo.toml");
        assert!(refusal.message().contains("there is no Cargo.toml at"));
        assert!(
            refusal
                .message()
                .contains("what cargo resolves from a manifest"),
            "{}",
            refusal.message()
        );
        sandbox.write("Cargo.toml", "[package]\nname = \"app\"\n");
        assert!(manifest_of(&sandbox.root).is_ok());
    }

    #[test]
    fn a_record_may_not_be_written_over_the_manifest_or_the_lockfile() {
        let sandbox = sandbox();
        sandbox.write("Cargo.toml", "[package]\nname = \"app\"\n");
        sandbox.write("Cargo.lock", "version = 4\n");

        let refusal = super::may_record_at(&sandbox.root, &sandbox.root.join("Cargo.toml"))
            .expect_err("the manifest is refused");
        assert!(matches!(
            refusal,
            super::DependencyRefusal::TheManifest { .. }
        ));
        assert!(refusal.message().contains("goes to its own file"));
        // However the path is spelled: the same file reached by another name is the same file.
        let spelled = sandbox.root.join("sub").join("..").join("Cargo.lock");
        std::fs::create_dir_all(sandbox.root.join("sub")).expect("a directory");
        let refusal =
            super::may_record_at(&sandbox.root, &spelled).expect_err("the lockfile is refused");
        assert!(matches!(
            refusal,
            super::DependencyRefusal::TheLockfile { .. }
        ));
        assert!(refusal.message().contains("is the lockfile"));
        // Anywhere else is fine, inside the directory or not.
        assert!(super::may_record_at(&sandbox.root, &sandbox.root.join("deps.json")).is_ok());
        assert!(super::may_record_at(&sandbox.root, &sandbox.root.join("sub/deps.json")).is_ok());
    }

    #[test]
    fn a_digest_reads_what_cargo_reported_and_keeps_it_small() {
        let digest =
            Digest::of_cargo_metadata(&one_member(), Path::new(NOWHERE)).expect("a report");

        assert_eq!(digest.root(), NOWHERE);
        assert_eq!(digest.metadata_version(), 1);
        assert_eq!(digest.packages().len(), 3);
        assert_eq!(digest.workspace_members().len(), 1);
        assert_eq!(digest.dependencies().len(), 2);
        let member = digest.workspace_members()[0];
        assert_eq!(member.name(), "app");
        assert!(member.is_member());
        assert_eq!(member.features(), ["default"]);
        let serde = digest
            .dependencies()
            .into_iter()
            .find(|package| package.name() == "serde")
            .expect("serde is a dependency");
        assert_eq!(serde.name(), "serde");
        assert_eq!(serde.version(), "1.0.200");
        assert!(serde.source().starts_with("registry+"));
        assert!(serde.features().is_empty());
        let rendered = digest.render().join("\n");
        assert!(rendered.contains("workspace members"), "{rendered}");
        assert!(rendered.contains("[member]"), "{rendered}");
        // A column that lines up must not leave the line ending in spaces: this output is quoted as
        // evidence, and trailing whitespace is a documentation check failure.
        assert!(
            digest.render().iter().all(|line| line == line.trim_end()),
            "{rendered}"
        );

        // What cargo did not say is not invented: `resolve` has no node for serde, so it is reported
        // with no features rather than with an empty list that means "we asked".
        let without_resolve = metadata(
            &package(MEMBER_ID, "app", "0.1.0", "null"),
            &format!("\"{MEMBER_ID}\""),
            "",
        )
        .replace(
            "\"resolve\": {\"nodes\": [], \"root\": null}",
            "\"resolve\": null",
        );
        let digest =
            Digest::of_cargo_metadata(&without_resolve, Path::new(NOWHERE)).expect("a report");
        assert!(digest.workspace_members()[0].features().is_empty());

        // A report that is not cargo's is refused rather than read as a small project.
        for wrong in [
            "{}",
            "{\"packages\": [], \"workspace_root\": \"/tmp/x\"}",
            "{\"packages\": [], \"workspace_members\": []}",
            "not json",
        ] {
            assert!(
                Digest::of_cargo_metadata(wrong, Path::new(NOWHERE)).is_err(),
                "{wrong} should be refused"
            );
        }
    }

    #[test]
    fn a_lockfile_says_its_version_and_how_many_packages_it_holds() {
        let sandbox = sandbox();
        assert_eq!(Lockfile::read(&sandbox.root), Lockfile::Absent);
        assert!(!Lockfile::read(&sandbox.root).is_present());

        sandbox.write(
            "Cargo.lock",
            "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.200\"\n",
        );
        let present = Lockfile::read(&sandbox.root);
        assert_eq!(present.packages(), Some(2));
        assert_eq!(
            present,
            Lockfile::Present {
                version: 4,
                packages: 2,
                bytes: std::fs::metadata(sandbox.root.join("Cargo.lock"))
                    .expect("the file")
                    .len() as usize
            }
        );
        let rendered = present.render();
        assert!(
            rendered.contains("Cargo.lock v4, 2 package(s)"),
            "{rendered}"
        );

        // A lockfile that cannot be read as text is a finding, not an empty one.
        std::fs::write(sandbox.root.join("Cargo.lock"), [0xff, 0xfe, 0xfd]).expect("bytes");
        assert!(matches!(
            Lockfile::read(&sandbox.root),
            Lockfile::Unreadable { .. }
        ));
        assert!(
            Lockfile::read(&sandbox.root)
                .render()
                .contains("could not be read")
        );
    }

    #[test]
    fn the_lockfile_is_read_where_cargo_keeps_it() {
        let sandbox = sandbox();
        sandbox.write("Cargo.lock", "version = 4\n\n[[package]]\nname = \"app\"\n");
        // A member directory: its manifest is inside a workspace, and the lockfile belongs to the
        // workspace root, so that is where the report reads it, not beside the manifest.
        let member = sandbox.root.join("crates").join("app");
        std::fs::create_dir_all(&member).expect("a member directory");
        let digest =
            Digest::of_cargo_metadata(&one_member_at(&sandbox.root), &member).expect("a report");
        assert_eq!(digest.root(), sandbox.root.to_string_lossy());
        assert_eq!(digest.lockfile().packages(), Some(1));

        // A report that names no root this machine has falls back to the directory that was named,
        // and says the truth about it rather than inventing a lockfile: no lockfile is a choice.
        let named = Digest::of_cargo_metadata(&one_member(), &sandbox.root).expect("a report");
        assert_eq!(named.root(), NOWHERE);
        assert_eq!(named.lockfile().packages(), Some(1));
        let empty = sandbox.root.join("no-lockfile-here");
        std::fs::create_dir_all(&empty).expect("a directory");
        let absent = Digest::of_cargo_metadata(&one_member(), &empty).expect("a report");
        assert_eq!(*absent.lockfile(), Lockfile::Absent);
    }

    #[test]
    fn a_review_names_what_changed_and_what_did_not() {
        let sandbox = sandbox();
        let two = "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
                   [[package]]\nname = \"serde\"\nversion = \"1.0.200\"\n";
        let four = "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
                    [[package]]\nname = \"serde\"\nversion = \"1.0.210\"\n\n\
                    [[package]]\nname = \"libc\"\nversion = \"0.2.155\"\n\n\
                    [[package]]\nname = \"smallvec\"\nversion = \"1.13.2\"\n";
        // The lockfile is read where cargo keeps it, so the fixture report is rooted at the sandbox
        // and the lockfile is a file in it.
        sandbox.write("Cargo.lock", two);
        let before = Digest::of_cargo_metadata(&one_member_at(&sandbox.root), &sandbox.root)
            .expect("a report");

        // A dependency added, one re-versioned, a feature changed on a member, and one dependency
        // exactly as it was.
        sandbox.write("Cargo.lock", four);
        let after_text = metadata_at(
            &sandbox.root,
            &format!(
                "{}, {}, {}, {}",
                package(MEMBER_ID, "app", "0.1.0", "null"),
                registry("serde", "1.0.210"),
                registry("libc", "0.2.155"),
                registry("smallvec", "1.13.2")
            ),
            &format!("\"{MEMBER_ID}\""),
            &format!(
                "{{\"id\": \"{MEMBER_ID}\", \"dependencies\": [\"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.210\"], \
                  \"deps\": [], \"features\": [\"default\", \"extra\"]}}"
            ),
        );
        let after = Digest::of_cargo_metadata(&after_text, &sandbox.root).expect("a report");
        assert_eq!(after.lockfile().packages(), Some(4));

        let review = Review::between(&before, &after).expect("the same workspace");
        assert_eq!(review.changes().len(), 3, "{:?}", review.render());
        assert_eq!(review.unchanged(), 1, "{:?}", review.render());
        assert!(review.lockfile_changed());
        let rendered = review.render().join("\n");
        assert!(rendered.contains("version  serde"), "{rendered}");
        assert!(rendered.contains("1.0.200 → 1.0.210"), "{rendered}");
        assert!(rendered.contains("added    smallvec"), "{rendered}");
        assert!(rendered.contains("features app"), "{rendered}");
        assert!(rendered.contains("lockfile:"), "{rendered}");
        // `libc` is in both reports at the same version, and it is counted rather than listed.
        assert!(
            rendered.contains("1 package(s) are as they were"),
            "{rendered}"
        );
        assert!(!rendered.contains("libc 0.2.155 → 0.2.155"), "{rendered}");
        assert!(!review.is_empty());

        // The same report twice changes nothing, and says so rather than printing an empty list.
        let quiet = Review::between(&after, &after).expect("the same workspace");
        assert!(quiet.is_empty());
        assert_eq!(quiet.changes().len(), 0);
        assert!(quiet.render()[0].contains("no package changed"));

        // A second bump, on top of that one: cargo names the version in the identifier, so the bump
        // is a removal and an addition that get paired into one version change.
        // The report to compare with has no lockfile: one report had one and the other did not, which
        // is a difference in the workspace and is reported as one.
        std::fs::remove_file(sandbox.root.join(LOCKFILE_FILE_NAME)).expect("the lockfile goes");
        let bumped = metadata_at(
            &sandbox.root,
            &format!(
                "{}, {}, {}, {}",
                package(MEMBER_ID, "app", "0.1.0", "null"),
                registry("serde", "1.0.211"),
                registry("libc", "0.2.155"),
                registry("smallvec", "1.13.2")
            ),
            &format!("\"{MEMBER_ID}\""),
            // The same features as the report it is compared with, so the only difference is the
            // version: a fixture that forgot them would report a feature change that never happened.
            &format!(
                "{{\"id\": \"{MEMBER_ID}\", \"dependencies\": [], \"deps\": [], \
                  \"features\": [\"default\", \"extra\"]}}"
            ),
        );
        let bumped = Digest::of_cargo_metadata(&bumped, &sandbox.root).expect("a report");
        assert_eq!(*bumped.lockfile(), Lockfile::Absent);
        let paired = Review::between(&after, &bumped).expect("the same workspace");
        assert_eq!(paired.changes().len(), 1, "{:?}", paired.render());
        assert_eq!(paired.changes()[0].kind(), super::ChangeKind::Version);
        assert_eq!(paired.changes()[0].from(), Some("1.0.210"));
        assert_eq!(paired.changes()[0].to(), Some("1.0.211"));
        assert!(
            paired.lockfile_changed(),
            "one report had a lockfile and one did not"
        );

        // An identifier that did not change while the version did does not happen in what cargo
        // prints, and the reader still does not call it unchanged: a review that quietly ignored a
        // version difference would be hiding exactly what it is for.
        let same_id = one_member_at(&sandbox.root)
            .replace("\"version\": \"1.0.200\"", "\"version\": \"1.0.201\"");
        let same_id = Digest::of_cargo_metadata(&same_id, &sandbox.root).expect("a report");
        let odd = Review::between(&before, &same_id).expect("the same workspace");
        assert_eq!(odd.changes().len(), 1, "{:?}", odd.render());
        assert!(odd.render().join("\n").contains("1.0.200 → 1.0.201"));

        // A review that loses two dependencies names both, and says the lockfile did not change:
        // the same lockfile is read for both reports.
        sandbox.write("Cargo.lock", two);
        let without = metadata_at(
            &sandbox.root,
            &package(MEMBER_ID, "app", "0.1.0", "null"),
            &format!("\"{MEMBER_ID}\""),
            &format!(
                "{{\"id\": \"{MEMBER_ID}\", \"dependencies\": [], \"deps\": [], \
                  \"features\": [\"default\"]}}"
            ),
        );
        let gone = Digest::of_cargo_metadata(&without, &sandbox.root).expect("a report");
        assert_eq!(gone.lockfile(), before.lockfile());
        let review = Review::between(&before, &gone).expect("the same workspace");
        assert_eq!(review.changes().len(), 2, "{:?}", review.render());
        assert!(
            review
                .changes()
                .iter()
                .all(|change| change.kind() == super::ChangeKind::Removed)
        );
        assert!(review.changes().iter().all(|change| !change.is_a_member()));
        assert!(
            review.render().join("\n").contains("removed  serde"),
            "{:?}",
            review.render()
        );
        assert!(!review.lockfile_changed());

        // A record of another workspace is refused rather than compared.
        let elsewhere = one_member_at(Path::new("/tmp/other"));
        let other = Digest::of_cargo_metadata(&elsewhere, Path::new(NOWHERE)).expect("a report");
        let refusal = Review::between(&before, &other).expect_err("another workspace");
        assert!(matches!(refusal, ReviewRefusal::AnotherWorkspace { .. }));
        assert!(refusal.message().contains("belongs to one workspace"));
        assert!(sandbox.root.exists());
    }

    #[test]
    fn a_digest_round_trips_through_a_record_and_refuses_a_foreign_one() {
        let sandbox = sandbox();
        sandbox.write(
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\n",
        );
        let digest = Digest::of_cargo_metadata(&one_member_at(&sandbox.root), &sandbox.root)
            .expect("a report");

        let path = sandbox.root.join("deps.json");
        let written = digest.save(&path).expect("the record is written");
        assert_eq!(written, path);
        let text = std::fs::read_to_string(&path).expect("the record");
        for member in RECORD_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        for member in super::PACKAGE_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        let reloaded = Digest::load(&path).expect("the record reads back");
        assert_eq!(reloaded, digest);
        assert_eq!(
            Review::between(&digest, &reloaded)
                .expect("same")
                .changes()
                .len(),
            0
        );

        // A record from a newer build is refused rather than half-read, and one of another workspace
        // is refused when it is used as a baseline.
        std::fs::write(&path, text.replace("\"format\": 1", "\"format\": 3"))
            .expect("a newer record");
        assert!(Digest::load(&path).is_err());
        std::fs::write(&path, "{\"format\": 1}").expect("a broken record");
        assert!(Digest::load(&path).is_err());
    }

    #[test]
    fn a_failed_resolution_keeps_its_reason() {
        let stderr = "    Updating crates.io index\n  error: the lock file /tmp/area/Cargo.lock needs to be updated but --locked was passed to prevent this\n";
        assert_eq!(
            super::failure_reason(stderr),
            Some(
                "error: the lock file /tmp/area/Cargo.lock needs to be updated but --locked was passed to prevent this"
                    .to_owned()
            )
        );
        let reason = super::failure_reason(stderr).expect("the reason");
        assert!(super::is_a_lockfile_disagreement(&reason));
        assert_eq!(super::failure_reason("all good"), None);
        // A reason that is not about the lockfile is not read as one.
        assert!(!super::is_a_lockfile_disagreement(
            "error: failed to parse manifest at `/tmp/area/Cargo.toml`"
        ));
        assert!(!super::is_a_lockfile_disagreement(
            "error: no such subcommand"
        ));
    }
}
