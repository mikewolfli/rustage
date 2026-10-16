// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! A kit: which tools a build uses, versioned, and what this machine has of them (task T02.10).
//!
//! The roadmap asks for a **versioned** tool combination — a toolchain, an SDK, a debugger, a device
//! and the environment — and for two things to be diagnosed rather than papered over: a member that
//! is not there, and a combination that cannot work. Both live here.
//!
//! Three decisions shape the module:
//!
//! * **A kit is not an output.** Which profile is being built and which target triple it is built for
//!   are answers [`crate::project::build::configuration`] and the target give; a kit says *which tools*, and
//!   the only thing a tool has an opinion about is the triple it can produce for. So a kit never
//!   carries a profile, and the question it is asked is a triple ([`Kit::diagnose`]). Members declare
//!   the triples they serve — explicitly, as "any triple" or as a list, never by leaving a field out —
//!   and "can this combination serve that output" is the intersection of those answers.
//! * **One member per role.** Two toolchains are not a combination, they are a question nobody
//!   answered, so a document that has two is refused when it is read rather than resolved later by
//!   whichever one the code happens to find first.
//! * **Not asked is not missing.** A member nobody asked the machine about is `not-checked` with the
//!   reason, a member whose probe found nothing is `missing`, and a member that was found but does
//!   not satisfy the kit is neither: its version is the finding. Collapsing those three is how a
//!   report starts telling people to install what they already have.
//!
//! Asking the machine is the host's business and not this module's: [`Observed`] is what somebody
//! else found (the toolchain check, the filesystem, the environment, a program's own output), and
//! [`version_like`] is the one parsing rule applied to what a program printed.

use std::{
    cmp::Ordering,
    fmt,
    path::{Path, PathBuf},
};

use crate::contracts::revision::FormatVersion;
use crate::host::paths::ProjectRoot;
use crate::project::lifecycle::session::{ToolUnavailable, ToolchainState};
use crate::project::record::format::{ProjectError, format_version};
use crate::project::record::json::{self, Json, JsonKind, JsonObject};

/// The document that says which tools a kit is made of.
pub const KIT_FILE_NAME: &str = "kit.json";

/// The version of a kit document this build writes.
pub const KIT_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The members a kit document has, in the order it is written.
pub const KIT_MEMBERS: [&str; 3] = ["format", "kit", "members"];

/// The members the kit's own identity has. A kit is versioned: two versions are two kits.
pub const IDENTITY_MEMBERS: [&str; 3] = ["id", "name", "version"];

/// The members every kit member has.
pub const MEMBER_MEMBERS: [&str; 4] = ["role", "name", "version", "serves"];

/// The member a kit member has when it says how to ask the machine.
pub const PROBE_MEMBER: &str = "probe";

/// What part a tool plays in a build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// The compiler and its cargo.
    Toolchain,
    /// A platform SDK the toolchain builds against.
    Sdk,
    /// A debugger that attaches to a run.
    Debugger,
    /// A device or emulator a build is put on.
    Device,
    /// Something in the environment: a variable, a directory, a flag.
    Environment,
}

impl Role {
    /// Every role, in the order a report lists them.
    pub const ALL: [Self; 5] = [
        Self::Toolchain,
        Self::Sdk,
        Self::Debugger,
        Self::Device,
        Self::Environment,
    ];

    /// The name a document and a report use.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Toolchain => "toolchain",
            Self::Sdk => "sdk",
            Self::Debugger => "debugger",
            Self::Device => "device",
            Self::Environment => "environment",
        }
    }

    /// The role a document names.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.name() == word)
    }

    /// Whether a build cannot happen at all without this role.
    ///
    /// Only the toolchain is required: a debugger, a device or an SDK is part of the task a person
    /// is doing, and calling a kit incomplete for not naming one would make every kit incomplete.
    /// A role that is not played is still reported, because a missing member is a finding.
    pub const fn is_required(self) -> bool {
        matches!(self, Self::Toolchain)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name())
    }
}

/// Why a version requirement could not be read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RequirementRefusal {
    /// The text says nothing.
    Empty,
    /// The text is not one of the three shapes this build understands.
    Shape {
        /// What was written.
        written: String,
    },
}

impl RequirementRefusal {
    /// A sentence for a person.
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "the version says nothing, and a member that says nothing about its \
                            version cannot be checked"
                .to_owned(),
            Self::Shape { written } => format!(
                "`{written}` is not a version this build understands; write `*`, `1.2.3`, or \
                 `>=1.2.3`"
            ),
        }
    }
}

impl fmt::Display for RequirementRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

impl std::error::Error for RequirementRefusal {}

/// What a member has to be.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Requirement {
    /// Anything will do, which is a thing to write down rather than to leave out.
    Any,
    /// Exactly this.
    Exact(String),
    /// This or newer.
    AtLeast(String),
}

impl Requirement {
    /// Reads a requirement: `*`, `1.2.3` or `>=1.2.3`.
    pub fn parse(text: &str) -> Result<Self, RequirementRefusal> {
        let text = text.trim();
        if text.is_empty() {
            return Err(RequirementRefusal::Empty);
        }
        if text == "*" {
            return Ok(Self::Any);
        }
        if let Some(rest) = text.strip_prefix(">=") {
            let rest = rest.trim();
            if rest.is_empty() {
                return Err(RequirementRefusal::Shape {
                    written: text.to_owned(),
                });
            }
            return Ok(Self::AtLeast(rest.to_owned()));
        }
        if text.contains(char::is_whitespace) || text.starts_with(['>', '<', '=', '^', '~']) {
            return Err(RequirementRefusal::Shape {
                written: text.to_owned(),
            });
        }
        Ok(Self::Exact(text.to_owned()))
    }

    /// The requirement as it is written.
    pub fn render(&self) -> String {
        match self {
            Self::Any => "any version".to_owned(),
            Self::Exact(text) => format!("exactly `{text}`"),
            Self::AtLeast(text) => format!("`{text}` or newer"),
        }
    }

    /// Whether a version found on this machine satisfies it.
    ///
    /// `None` is the answer for a version this build cannot compare — `1.99.0-nightly` and
    /// `apple-1.0` are not dotted numbers, and both "it holds" and "it does not hold" would be
    /// invented. The finding for that is `not-checked`, with the version quoted.
    pub fn holds(&self, found: &str) -> Option<bool> {
        match self {
            Self::Any => Some(true),
            // Exactly this, compared as numbers when both are dotted numbers and as text when they
            // are not: `v1.98` is not `1.98`, and saying so is an answer rather than a shrug.
            Self::Exact(wanted) => match compare(found, wanted) {
                Some(ordering) => Some(ordering == Ordering::Equal),
                None => Some(found == wanted),
            },
            Self::AtLeast(wanted) => {
                compare(found, wanted).map(|ordering| ordering != Ordering::Less)
            }
        }
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.render())
    }
}

/// Compares two dotted numbers, and refuses anything else.
fn compare(found: &str, wanted: &str) -> Option<Ordering> {
    let parts = |text: &str| -> Option<Vec<u64>> {
        let pieces: Vec<&str> = text.split('.').collect();
        if pieces.is_empty() {
            return None;
        }
        pieces
            .iter()
            .map(|piece| piece.parse::<u64>().ok())
            .collect::<Option<Vec<u64>>>()
    };
    let (a, b) = (parts(found)?, parts(wanted)?);
    for index in 0..a.len().max(b.len()) {
        let left = a.get(index).copied().unwrap_or(0);
        let right = b.get(index).copied().unwrap_or(0);
        match left.cmp(&right) {
            Ordering::Equal => {}
            other => return Some(other),
        }
    }
    Some(Ordering::Equal)
}

/// Which output triples a member can serve.
///
/// It is a value with two shapes rather than an optional list, because "no triples" and "every
/// triple" are different answers and an empty list would have to be read as one of them silently.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Serves {
    /// Any triple: the tool does not care what it produces for.
    Every,
    /// These triples, and no others.
    These(Vec<String>),
}

impl Serves {
    /// Whether this serves an output triple.
    pub fn serves(&self, triple: &str) -> bool {
        match self {
            Self::Every => true,
            Self::These(triples) => triples.iter().any(|there| there == triple),
        }
    }

    /// Whether two members could ever be used for one output.
    ///
    /// An empty intersection is not a judgement about either tool: it says that no single output can
    /// be served by both, which is what an incompatible combination is.
    pub fn overlaps(&self, other: &Serves) -> bool {
        match (self, other) {
            (Self::Every, _) | (_, Self::Every) => true,
            (Self::These(mine), Self::These(theirs)) => {
                mine.iter().any(|triple| theirs.contains(triple))
            }
        }
    }

    /// The triples, for a report.
    pub fn render(&self) -> String {
        match self {
            Self::Every => "any triple".to_owned(),
            Self::These(triples) => match triples.as_slice() {
                [] => "no triple at all".to_owned(),
                [one] => one.clone(),
                many => format!("{} triples", many.len()),
            },
        }
    }
}

/// How to ask this machine what it has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Probe {
    /// A program, and what to ask it. Its own output is read by {@link version_like}.
    Program {
        /// The program.
        command: String,
        /// The argument it is asked with, when the kit names one.
        argument: Option<String>,
    },
    /// A path that has to be there, resolved against the kit's own directory.
    Path {
        /// The path as the kit writes it.
        written: String,
    },
    /// An environment variable that has to be set.
    Variable {
        /// The variable's name.
        name: String,
    },
    /// The kit does not say how to find out, and says why.
    Undetermined {
        /// Why not.
        reason: String,
    },
}

impl Probe {
    /// What the probe is, in a person's words.
    pub fn render(&self) -> String {
        match self {
            Self::Program { command, argument } => match argument {
                Some(argument) => format!("`{command} {argument}`"),
                None => format!("`{command}`"),
            },
            Self::Path { written } => format!("the path `{written}`"),
            Self::Variable { name } => format!("the variable `{name}`"),
            Self::Undetermined { .. } => "nothing: the kit does not say how to find it".to_owned(),
        }
    }
}

/// One tool a kit is made of.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KitMember {
    role: Role,
    name: String,
    version: Requirement,
    serves: Serves,
    probe: Probe,
}

impl KitMember {
    /// What part it plays.
    pub const fn role(&self) -> Role {
        self.role
    }

    /// What it is called.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it has to be.
    pub const fn version(&self) -> &Requirement {
        &self.version
    }

    /// What it can serve.
    pub const fn serves(&self) -> &Serves {
        &self.serves
    }

    /// How to ask this machine about it.
    pub const fn probe(&self) -> &Probe {
        &self.probe
    }

    /// One line: the role, the name, the version and what it serves.
    pub fn render(&self) -> String {
        format!(
            "{:<12} {:<24} {:<20} serves {:<24} ask {}",
            self.role.name(),
            self.name,
            self.version.render(),
            self.serves.render(),
            self.probe.render()
        )
    }
}

/// A kit document that was read.
#[derive(Clone, Debug)]
pub struct Kit {
    path: PathBuf,
    root: ProjectRoot,
    id: String,
    name: String,
    version: String,
    members: Vec<KitMember>,
}

impl Kit {
    /// Reads a kit document.
    pub fn read(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let text = std::fs::read_to_string(&path).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "reading the kit document",
            reason: error.to_string(),
        })?;
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let root = ProjectRoot::new(&directory).map_err(|error| ProjectError::Io {
            path: path.clone(),
            action: "resolving the directory the kit is in",
            reason: error.message(),
        })?;
        Self::interpret(path, root, &text)
    }

    fn interpret(path: PathBuf, root: ProjectRoot, text: &str) -> Result<Self, ProjectError> {
        let value = json::parse(text).map_err(|error| ProjectError::NotJson {
            path: path.clone(),
            error,
        })?;
        let version = format_version(&value)?;
        match version.get().cmp(&KIT_FORMAT.get()) {
            Ordering::Equal => {}
            Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: KIT_FORMAT,
                });
            }
            Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: KIT_FORMAT,
                });
            }
        }
        let Some(object) = value.as_object() else {
            return Err(ProjectError::WrongType {
                path: "the kit document".to_owned(),
                expected: "an object",
                found: value.kind(),
            });
        };
        for member in KIT_MEMBERS {
            if !object.contains(member) {
                return Err(ProjectError::MissingMember {
                    path: format!("`{member}` in the kit document"),
                });
            }
        }
        refuse_unknown(object, &KIT_MEMBERS, "the kit document")?;
        let identity =
            object
                .get("kit")
                .and_then(Json::as_object)
                .ok_or_else(|| ProjectError::WrongType {
                    path: "`kit`".to_owned(),
                    expected: "an object with the kit's identity",
                    found: object.get("kit").map(Json::kind).unwrap_or(JsonKind::Null),
                })?;
        for member in IDENTITY_MEMBERS {
            if identity.get(member).and_then(Json::as_str).is_none() {
                return Err(ProjectError::MissingMember {
                    path: format!("`kit.{member}`"),
                });
            }
        }
        let id = text_member(identity, "id", "kit.id")?;
        let name = text_member(identity, "name", "kit.name")?;
        let kit_version = text_member(identity, "version", "kit.version")?;
        if kit_version.trim().is_empty() {
            return Err(ProjectError::EmptyValue {
                path: "`kit.version`".to_owned(),
            });
        }
        let Some(entries) = object.get("members").and_then(Json::as_array) else {
            return Err(ProjectError::WrongType {
                path: "`members`".to_owned(),
                expected: "an array",
                found: object
                    .get("members")
                    .map(Json::kind)
                    .unwrap_or(JsonKind::Null),
            });
        };

        let mut members: Vec<KitMember> = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let at = format!("members[{index}]");
            let Some(entry) = entry.as_object() else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}`"),
                    expected: "an object",
                    found: entry.kind(),
                });
            };
            for member in MEMBER_MEMBERS {
                if entry.get(member).is_none() {
                    return Err(ProjectError::MissingMember {
                        path: format!("`{at}.{member}`"),
                    });
                }
            }
            let mut allowed = MEMBER_MEMBERS.to_vec();
            allowed.push(PROBE_MEMBER);
            refuse_unknown(entry, &allowed, &at)?;
            let role_word = text_member(entry, "role", &format!("{at}.role"))?;
            let Some(role) = Role::parse(&role_word) else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}.role`"),
                    expected: "`toolchain`, `sdk`, `debugger`, `device` or `environment`",
                    found: JsonKind::String,
                });
            };
            let member_name = text_member(entry, "name", &format!("{at}.name"))?;
            if member_name.trim().is_empty() {
                return Err(ProjectError::EmptyValue {
                    path: format!("`{at}.name`"),
                });
            }
            // One member per role: two toolchains are a question, not a combination, and resolving
            // it by whichever one is found first is exactly the silent choice this forbids.
            if members.iter().any(|there| there.role == role) {
                return Err(ProjectError::Duplicate {
                    path: format!("`{at}.role`"),
                    claim: role_word,
                });
            }
            let requirement =
                Requirement::parse(&text_member(entry, "version", &format!("{at}.version"))?)
                    .map_err(|refusal| ProjectError::Inconsistent {
                        path: format!("`{at}.version`"),
                        reason: refusal.message(),
                    })?;
            let serves = read_serves(entry, &at)?;
            let probe = read_probe(entry, &at, &root)?;
            // A path can say whether something is there and cannot say which version it is, so a
            // version requirement beside a path probe is refused rather than silently satisfied:
            // `exactly 1.0` next to a directory would otherwise read as a check that never happened.
            if matches!(probe, Probe::Path { .. }) && !matches!(requirement, Requirement::Any) {
                return Err(ProjectError::Inconsistent {
                    path: format!("`{at}.version`"),
                    reason: format!(
                        "{} asks about a path, and a path cannot say which version it is; write \
                         `*` or ask a program",
                        probe.render()
                    ),
                });
            }
            members.push(KitMember {
                role,
                name: member_name,
                version: requirement,
                serves,
                probe,
            });
        }

        Ok(Self {
            path,
            root,
            id,
            name,
            version: kit_version,
            members,
        })
    }

    /// Where the kit document is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The directory a member's paths are relative to.
    pub fn root(&self) -> &ProjectRoot {
        &self.root
    }

    /// The kit's identity.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The kit's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The kit's own version: two versions are two kits, and a build records which one it used.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The members, in document order.
    pub fn members(&self) -> &[KitMember] {
        &self.members
    }

    /// The member that plays a role, when one does.
    pub fn member(&self, role: Role) -> Option<&KitMember> {
        self.members.iter().find(|member| member.role == role)
    }

    /// The roles no member plays.
    pub fn roles_without_a_member(&self) -> Vec<Role> {
        Role::ALL
            .into_iter()
            .filter(|role| self.member(*role).is_none())
            .collect()
    }

    /// What the kit makes of this machine, and of an output triple when one was asked about.
    ///
    /// The triple is the only thing about the output a tool is asked: which profile is being built
    /// is not a tool's business, and a kit that carried one would be answering a question nobody
    /// asked it.
    pub fn diagnose(&self, observed: &Observed, triple: Option<&str>) -> Diagnosis {
        let mut findings = Vec::new();
        for member in &self.members {
            let finding = match (member.serves.serves(triple.unwrap_or_default()), triple) {
                (false, Some(triple)) => Finding::CannotServe {
                    triple: triple.to_owned(),
                    serves: member.serves.render(),
                },
                _ => match observed.of(&member.name) {
                    Found::Value(value) => match member.version.holds(value.as_str()) {
                        Some(true) => Finding::Ok {
                            found: value.clone(),
                        },
                        Some(false) => Finding::Version {
                            found: value.clone(),
                            required: member.version.render(),
                        },
                        None => Finding::NotChecked {
                            reason: format!(
                                "`{value}` is not a dotted number, so `{}` cannot be decided",
                                member.version.render()
                            ),
                        },
                    },
                    Found::Nothing { reason } => Finding::Missing {
                        reason: reason.clone(),
                    },
                    Found::NotChecked { reason } => Finding::NotChecked {
                        reason: reason.clone(),
                    },
                },
            };
            findings.push(Diagnosed {
                role: member.role,
                member: member.name.clone(),
                finding,
            });
        }

        let mut incompatible = Vec::new();
        for (index, one) in self.members.iter().enumerate() {
            for other in &self.members[index + 1..] {
                if !one.serves.overlaps(&other.serves) {
                    incompatible.push(format!(
                        "`{} {}` serves {} and `{} {}` serves {}, and the two have no output triple \
                         in common",
                        one.role.name(),
                        one.name,
                        one.serves.render(),
                        other.role.name(),
                        other.name,
                        other.serves.render()
                    ));
                }
            }
        }

        Diagnosis {
            kit: self.name.clone(),
            version: self.version.clone(),
            triple: triple.map(str::to_owned),
            findings,
            without_a_member: self.roles_without_a_member(),
            incompatible,
        }
    }

    /// The document as JSON.
    pub fn to_json(&self) -> Json {
        let mut identity = JsonObject::new();
        identity.insert("id", Json::string(&self.id));
        identity.insert("name", Json::string(&self.name));
        identity.insert("version", Json::string(&self.version));
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(KIT_FORMAT.get())));
        out.insert("kit", Json::Object(identity));
        out.insert(
            "members",
            Json::array(
                self.members
                    .iter()
                    .map(|member| {
                        let mut entry = JsonObject::new();
                        entry.insert("role", Json::string(member.role.name()));
                        entry.insert("name", Json::string(&member.name));
                        let required = match &member.version {
                            Requirement::Any => "*".to_owned(),
                            Requirement::Exact(text) => text.clone(),
                            Requirement::AtLeast(text) => format!(">={text}"),
                        };
                        entry.insert("version", Json::string(required));
                        entry.insert(
                            "serves",
                            match &member.serves {
                                Serves::Every => Json::string("*"),
                                Serves::These(triples) => Json::array(
                                    triples.iter().map(Json::string).collect::<Vec<_>>(),
                                ),
                            },
                        );
                        match &member.probe {
                            Probe::Program { command, argument } => {
                                let mut probe = JsonObject::new();
                                probe.insert("kind", Json::string("program"));
                                probe.insert("command", Json::string(command));
                                if let Some(argument) = argument {
                                    probe.insert("argument", Json::string(argument));
                                }
                                entry.insert("probe", Json::Object(probe));
                            }
                            Probe::Path { written } => {
                                let mut probe = JsonObject::new();
                                probe.insert("kind", Json::string("path"));
                                probe.insert("path", Json::string(written));
                                entry.insert("probe", Json::Object(probe));
                            }
                            Probe::Variable { name } => {
                                let mut probe = JsonObject::new();
                                probe.insert("kind", Json::string("variable"));
                                probe.insert("variable", Json::string(name));
                                entry.insert("probe", Json::Object(probe));
                            }
                            Probe::Undetermined { reason } => {
                                let mut probe = JsonObject::new();
                                probe.insert("kind", Json::string("undetermined"));
                                probe.insert("reason", Json::string(reason));
                                entry.insert("probe", Json::Object(probe));
                            }
                        }
                        Json::Object(entry)
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        Json::Object(out)
    }

    /// One line per member, then the roles no member plays.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "kit `{}` v{} ({}), {} member(s), root {}",
            self.name,
            self.version,
            self.id,
            self.members.len(),
            self.root.as_path().display()
        )];
        for member in &self.members {
            lines.push(format!("  {}", member.render()));
        }
        let empty = self.roles_without_a_member();
        if empty.is_empty() {
            lines.push("every role has a member".to_owned());
        } else {
            lines.push(format!(
                "no member plays: {}",
                empty
                    .iter()
                    .map(|role| role.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        lines
    }
}

/// Refuses members this build does not know.
///
/// A kit is the host's own document, so an unknown member is a mistake rather than somebody else's
/// extension to preserve — and it is refused rather than ignored because ignoring it would let a
/// document that says `"profile": "desktop"` read as if that had been understood. Which profile is
/// being built belongs to the project's configuration and never to a kit.
fn refuse_unknown(object: &JsonObject, allowed: &[&str], at: &str) -> Result<(), ProjectError> {
    for name in object.names() {
        if !allowed.contains(&name) {
            return Err(ProjectError::Inconsistent {
                path: format!("`{at}.{name}`"),
                reason: format!(
                    "`{name}` is not a member a kit understands (a kit names tools, and which \
                     profile is being built belongs to the project's configuration)"
                ),
            });
        }
    }
    Ok(())
}

fn text_member(object: &JsonObject, member: &str, where_: &str) -> Result<String, ProjectError> {
    object
        .get(member)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProjectError::MissingMember {
            path: format!("`{where_}`"),
        })
}

fn read_serves(entry: &JsonObject, at: &str) -> Result<Serves, ProjectError> {
    match entry.get("serves") {
        Some(value) if value.as_str() == Some("*") => Ok(Serves::Every),
        Some(value) => {
            let Some(triples) = value.as_array() else {
                return Err(ProjectError::WrongType {
                    path: format!("`{at}.serves`"),
                    expected: "`*` or an array of triples",
                    found: value.kind(),
                });
            };
            let mut out = Vec::new();
            for (index, triple) in triples.iter().enumerate() {
                let Some(text) = triple.as_str() else {
                    return Err(ProjectError::WrongType {
                        path: format!("`{at}.serves[{index}]`"),
                        expected: "a target triple",
                        found: triple.kind(),
                    });
                };
                if text.trim().is_empty() {
                    return Err(ProjectError::EmptyValue {
                        path: format!("`{at}.serves[{index}]`"),
                    });
                }
                out.push(text.to_owned());
            }
            Ok(Serves::These(out))
        }
        None => Err(ProjectError::MissingMember {
            path: format!("`{at}.serves`"),
        }),
    }
}

fn read_probe(entry: &JsonObject, at: &str, root: &ProjectRoot) -> Result<Probe, ProjectError> {
    let Some(value) = entry.get(PROBE_MEMBER) else {
        // A member without a probe is not an error: the kit says it needs the tool and does not say
        // how to find it, which is a `not-checked` finding rather than a document refusal.
        return Ok(Probe::Undetermined {
            reason: "the kit does not say how to find this one".to_owned(),
        });
    };
    let Some(probe) = value.as_object() else {
        return Err(ProjectError::WrongType {
            path: format!("`{at}.{PROBE_MEMBER}`"),
            expected: "an object",
            found: value.kind(),
        });
    };
    let at_probe = format!("{at}.{PROBE_MEMBER}");
    let kind = text_member(probe, "kind", &format!("{at_probe}.kind"))?;
    match kind.as_str() {
        "program" => Ok(Probe::Program {
            command: text_member(probe, "command", &format!("{at_probe}.command"))?,
            argument: probe
                .get("argument")
                .and_then(Json::as_str)
                .map(str::to_owned),
        }),
        "path" => {
            let written = text_member(probe, "path", &format!("{at_probe}.path"))?;
            // The path is resolved against the kit's own directory with the project's own rule, so a
            // probe cannot reach out of the directory the kit is in.
            if root.resolve_for_creation(&written).is_err() {
                return Err(ProjectError::Inconsistent {
                    path: format!("`{at_probe}.path`"),
                    reason: format!("`{written}` does not stay inside the directory the kit is in"),
                });
            }
            Ok(Probe::Path { written })
        }
        "variable" => Ok(Probe::Variable {
            name: text_member(probe, "variable", &format!("{at_probe}.variable"))?,
        }),
        "undetermined" => Ok(Probe::Undetermined {
            reason: text_member(probe, "reason", &format!("{at_probe}.reason"))?,
        }),
        _ => Err(ProjectError::WrongType {
            path: format!("`{at_probe}.kind`"),
            expected: "`program`, `path`, `variable` or `undetermined`",
            found: JsonKind::String,
        }),
    }
}

/// What a probe found, or why nobody looked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Found {
    /// Something was found, and this is what it says it is.
    Value(String),
    /// Somebody looked and found nothing.
    Nothing {
        /// Where they looked.
        reason: String,
    },
    /// Nobody looked, and why.
    NotChecked {
        /// What stopped the look.
        reason: String,
    },
}

/// What this machine has, as far as somebody asked.
///
/// It is built by the caller because asking means running programs and reading the filesystem, and
/// this module does not do either; a member nobody asked about is `not-checked`, never `missing`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Observed {
    entries: Vec<(String, Found)>,
}

impl Observed {
    /// Nothing was asked yet, which is what a report with no diagnosis starts from.
    pub fn nothing() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// What was found about a member.
    pub fn with(mut self, member: &str, found: Found) -> Self {
        self.entries.retain(|(name, _)| name != member);
        self.entries.push((member.to_owned(), found));
        self
    }

    /// What was found about a member: `not-checked` when nobody asked.
    pub fn of(&self, member: &str) -> Found {
        self.entries
            .iter()
            .find(|(name, _)| name == member)
            .map(|(_, found)| found.clone())
            .unwrap_or_else(|| Found::NotChecked {
                reason: "nobody asked this machine about it".to_owned(),
            })
    }

    /// What a toolchain check already found, for the two programs it reads.
    ///
    /// It is a translation and not a second check: the versions are the ones
    /// [`ToolchainState`] reports, including the fact that it was not read at all.
    pub fn from_toolchain(state: &ToolchainState) -> Self {
        let mut observed = Self::nothing();
        let report = match state {
            ToolchainState::Checked(report) => report,
            ToolchainState::NotChecked(reason) => {
                for program in ["cargo", "rustc"] {
                    observed = observed.with(
                        program,
                        Found::NotChecked {
                            reason: reason.message(),
                        },
                    );
                }
                return observed;
            }
        };
        for (program, version) in [("cargo", report.cargo()), ("rustc", report.rustc())] {
            let found = match version {
                Some(version) => Found::Value(version.version().to_owned()),
                // The check's own typed finding is the reason: it knows which of "not installed",
                // "ran and said nothing" or "hung" happened, and a sentence written here would be a
                // second, drifting answer to a question already answered (principle #54).
                None => Found::Nothing {
                    reason: report
                        .unavailable(program)
                        .map(ToolUnavailable::line)
                        .unwrap_or_else(|| format!("`{program}` was run and reported no version")),
                },
            };
            observed = observed.with(program, found);
        }
        observed
    }
}

/// What the kit makes of one member.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Finding {
    /// It is there and satisfies what the kit asks.
    Ok {
        /// The version that was found.
        found: String,
    },
    /// It is not there.
    Missing {
        /// Where somebody looked.
        reason: String,
    },
    /// It is there and is not what the kit asks for.
    Version {
        /// The version that was found.
        found: String,
        /// What the kit asks for.
        required: String,
    },
    /// Nobody looked, and why.
    NotChecked {
        /// What stopped the look.
        reason: String,
    },
    /// It cannot serve the output being asked about.
    CannotServe {
        /// The triple that was asked about.
        triple: String,
        /// What the member can serve instead.
        serves: String,
    },
}

impl Finding {
    /// Whether this is a finding that stops the kit being usable.
    pub const fn is_problem(&self) -> bool {
        !matches!(self, Self::Ok { .. })
    }

    /// The word a report uses.
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Ok { .. } => "present",
            Self::Missing { .. } => "missing",
            Self::Version { .. } => "not the version the kit asks for",
            Self::NotChecked { .. } => "not checked",
            Self::CannotServe { .. } => "cannot serve",
        }
    }

    /// What to do about it, when there is something.
    pub fn render(&self) -> String {
        match self {
            Self::Ok { found } => format!("found `{found}`"),
            Self::Missing { reason } => format!("missing — {reason}"),
            Self::Version { found, required } => {
                format!("found `{found}`, and the kit asks for {required}")
            }
            Self::NotChecked { reason } => format!("not checked — {reason}"),
            Self::CannotServe { triple, serves } => {
                format!("it cannot serve `{triple}`; it serves {serves}")
            }
        }
    }
}

/// One member, and what the kit makes of it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnosed {
    role: Role,
    member: String,
    finding: Finding,
}

impl Diagnosed {
    /// What part it plays.
    pub const fn role(&self) -> Role {
        self.role
    }

    /// What it is called.
    pub fn member(&self) -> &str {
        &self.member
    }

    /// What was found.
    pub const fn finding(&self) -> &Finding {
        &self.finding
    }

    /// One line.
    pub fn render(&self) -> String {
        format!(
            "{:<12} {:<24} {:<26} {}",
            self.role.name(),
            self.member,
            self.finding.word(),
            self.finding.render()
        )
    }
}

/// What a kit makes of this machine, and of one output.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnosis {
    kit: String,
    version: String,
    triple: Option<String>,
    findings: Vec<Diagnosed>,
    without_a_member: Vec<Role>,
    incompatible: Vec<String>,
}

impl Diagnosis {
    /// The kit this is about.
    pub fn kit(&self) -> &str {
        &self.kit
    }

    /// The kit's version, which is part of what was diagnosed.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The output triple that was asked about, when one was.
    pub fn triple(&self) -> Option<&str> {
        self.triple.as_deref()
    }

    /// What was found about every member.
    pub fn findings(&self) -> &[Diagnosed] {
        &self.findings
    }

    /// The roles no member plays.
    pub fn roles_without_a_member(&self) -> &[Role] {
        &self.without_a_member
    }

    /// The combinations that can never serve one output.
    pub fn incompatible(&self) -> &[String] {
        &self.incompatible
    }

    /// The members with a finding that is not `present`.
    pub fn problems(&self) -> Vec<&Diagnosed> {
        self.findings
            .iter()
            .filter(|finding| finding.finding.is_problem())
            .collect()
    }

    /// Whether every member is there, matches, and can serve the output; and whether every required
    /// role is played.
    pub fn is_complete(&self) -> bool {
        self.problems().is_empty()
            && self.incompatible.is_empty()
            && !self.without_a_member.iter().any(|role| role.is_required())
    }

    /// Whether anything was **decided** to be wrong.
    ///
    /// It is not the same question as [`Self::is_complete`]: a member nobody asked about leaves a
    /// kit incomplete and is not a reason to report a failure to a script, which is the difference
    /// between "this kit cannot work" and "nobody looked".
    pub fn has_decided_problems(&self) -> bool {
        self.findings.iter().any(|finding| {
            !matches!(
                finding.finding,
                Finding::Ok { .. } | Finding::NotChecked { .. }
            )
        }) || !self.incompatible.is_empty()
            || self.without_a_member.iter().any(|role| role.is_required())
    }

    /// The report: what was asked, what each member is, and what stops the kit.
    ///
    /// A diagnosis that says nothing about a role and a diagnosis that found the role missing look
    /// the same in a list of findings, so the empty roles are printed even when they are not
    /// problems.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("kit `{}` v{}", self.kit, self.version)];
        match self.triple.as_deref() {
            Some(triple) => lines.push(format!(
                "output asked about: `{triple}` (the only thing about an output a tool is asked; \
                 which profile is being built belongs to the project's configuration)"
            )),
            None => lines.push(
                "no output triple was asked about, so whether the members can serve one is not \
                 checked"
                    .to_owned(),
            ),
        }
        for finding in &self.findings {
            lines.push(format!("  {}", finding.render()));
        }
        if self.without_a_member.is_empty() {
            lines.push("every role has a member".to_owned());
        } else {
            for role in &self.without_a_member {
                lines.push(format!(
                    "  no member plays the {} role{}",
                    role.name(),
                    if role.is_required() {
                        ", and nothing can be built without one"
                    } else {
                        ""
                    }
                ));
            }
        }
        for incompatible in &self.incompatible {
            lines.push(format!("  incompatible: {incompatible}"));
        }
        lines.push(format!(
            "kit is {}",
            if self.is_complete() {
                "complete for this output"
            } else {
                "not complete: a missing member and an incompatible combination are findings to act \
                 on, not a fallback"
            }
        ));
        lines
    }
}

/// The first thing in a program's output that looks like a version.
///
/// It is one rule, stated and tested, rather than each caller guessing: **the first run of digits and
/// dots that holds a dot**. `cargo 1.99.0 (5f94df478 2026-08-27)` gives `1.99.0`; a date gives
/// nothing, because none of its runs hold a dot; and `v1.2.3` gives `1.2.3`, because a `v` in front
/// of a version is ordinary. A program whose output has no such run is read as having no version,
/// which is a finding rather than a guess.
pub fn version_like(output: &str) -> Option<String> {
    let bytes: Vec<char> = output.chars().collect();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        let mut end = index;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == '.') {
            end += 1;
        }
        let candidate: String = bytes[start..end].iter().collect();
        let trimmed = candidate.trim_end_matches('.');
        if trimmed.contains('.') && !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
        index = end.max(index + 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::{
        Found, KIT_MEMBERS, Kit, MEMBER_MEMBERS, Observed, Requirement, RequirementRefusal, Role,
        Serves, version_like,
    };
    use crate::project::lifecycle::session::{
        NotChecked, ToolUnavailable, ToolVersion, ToolchainReport, ToolchainState,
    };

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
        let root = std::env::temp_dir().join(format!("rustage-kit-{}-{id}", std::process::id()));
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

        fn kit(&self, text: &str) -> Kit {
            let path = self.write("kit.json", text);
            Kit::read(&path).expect("the kit reads")
        }
    }

    /// A kit that names all five roles, with the toolchain asked about as a program and the others
    /// asked about in the ways they can be asked about.
    const KIT: &str = "{\"format\": 1, \
         \"kit\": {\"id\": \"kit-0001\", \"name\": \"Desktop tools\", \"version\": \"3\"}, \
         \"members\": [ \
           {\"role\": \"toolchain\", \"name\": \"rust\", \"version\": \">=1.98\", \"serves\": \"*\", \
            \"probe\": {\"kind\": \"program\", \"command\": \"cargo\", \"argument\": \"--version\"}}, \
           {\"role\": \"sdk\", \"name\": \"linux-sysroot\", \"version\": \"*\", \
            \"serves\": [\"x86_64-unknown-linux-gnu\"], \
            \"probe\": {\"kind\": \"path\", \"path\": \"sysroot\"}}, \
           {\"role\": \"debugger\", \"name\": \"lldb\", \"version\": \"*\", \
            \"serves\": [\"x86_64-unknown-linux-gnu\", \"aarch64-unknown-linux-gnu\"], \
            \"probe\": {\"kind\": \"program\", \"command\": \"lldb\", \"argument\": \"--version\"}}, \
           {\"role\": \"device\", \"name\": \"bench\", \"version\": \"*\", \
            \"serves\": [\"x86_64-unknown-linux-gnu\"], \
            \"probe\": {\"kind\": \"undetermined\", \"reason\": \"a device has to be named by a person\"}}, \
           {\"role\": \"environment\", \"name\": \"RUSTUP_TOOLCHAIN\", \"version\": \"*\", \
            \"serves\": \"*\", \"probe\": {\"kind\": \"variable\", \"variable\": \"RUSTUP_TOOLCHAIN\"}} \
         ]}";

    #[test]
    fn a_kit_reads_its_members_and_what_each_one_is_for() {
        let sandbox = sandbox();
        let kit = sandbox.kit(KIT);

        assert_eq!(kit.id(), "kit-0001");
        assert_eq!(kit.name(), "Desktop tools");
        assert_eq!(kit.version(), "3");
        assert_eq!(kit.members().len(), 5);
        let toolchain = kit.member(Role::Toolchain).expect("a member");
        assert_eq!(toolchain.name(), "rust");
        assert_eq!(
            toolchain.version(),
            &Requirement::AtLeast("1.98".to_owned())
        );
        assert_eq!(toolchain.serves(), &Serves::Every);
        assert!(kit.roles_without_a_member().is_empty());
        let rendered = kit.render().join("\n");
        assert!(
            rendered.contains("kit `Desktop tools` v3 (kit-0001)"),
            "{rendered}"
        );
        assert!(rendered.contains("every role has a member"), "{rendered}");
        assert!(rendered.contains("serves any triple"), "{rendered}");

        // A document that writes what a member is without saying how to find it is read, and the
        // member is reported as not checkable rather than as missing.
        let quiet = sandbox.kit(
            &KIT.replace(",\n            \"probe\": {\"kind\": \"program\", \"command\": \"cargo\", \"argument\": \"--version\"}", "")
                .replace(",            \"probe\": {\"kind\": \"program\", \"command\": \"cargo\", \"argument\": \"--version\"}", "")
                .replace(", \"probe\": {\"kind\": \"program\", \"command\": \"cargo\", \"argument\": \"--version\"}", ""),
        );
        assert_eq!(quiet.members().len(), 5);
        let diagnosis = quiet.diagnose(&Observed::nothing(), Some("x86_64-unknown-linux-gnu"));
        assert!(matches!(
            diagnosis.findings()[0].finding(),
            super::Finding::NotChecked { .. }
        ));
    }

    #[test]
    fn a_version_requirement_is_one_of_three_shapes_or_a_refusal() {
        assert_eq!(Requirement::parse("*"), Ok(Requirement::Any));
        assert_eq!(
            Requirement::parse("1.98.1"),
            Ok(Requirement::Exact("1.98.1".to_owned()))
        );
        assert_eq!(
            Requirement::parse(">= 1.98"),
            Ok(Requirement::AtLeast("1.98".to_owned()))
        );
        for bad in ["", "  ", ">=", "^1.2", "1.2 3"] {
            assert!(
                Requirement::parse(bad).is_err(),
                "{bad:?} should be refused"
            );
        }
        assert_eq!(
            Requirement::parse("~1").expect_err("refused"),
            RequirementRefusal::Shape {
                written: "~1".to_owned()
            }
        );
        assert!(
            Requirement::parse("")
                .expect_err("refused")
                .message()
                .contains("says nothing")
        );

        // Dotted numbers compare as numbers, not as text: 1.100 is newer than 1.99.
        assert_eq!(
            Requirement::AtLeast("1.99.0".to_owned()).holds("1.100.0"),
            Some(true)
        );
        assert_eq!(
            Requirement::AtLeast("1.99.0".to_owned()).holds("1.98.1"),
            Some(false)
        );
        assert_eq!(
            Requirement::Exact("1.98.1".to_owned()).holds("1.98.10"),
            Some(false)
        );
        assert_eq!(Requirement::Any.holds("anything at all"), Some(true));
        // A version this build cannot compare is not "met" and not "unmet".
        assert_eq!(
            Requirement::AtLeast("1.98".to_owned()).holds("1.99.0-nightly"),
            None
        );
        // `exact` falls back to comparing text, because "v1.98 is not 1.98" is an answer; only
        // `at least` has nothing to say about a version it cannot order.
        assert_eq!(
            Requirement::Exact("1.98".to_owned()).holds("v1.98"),
            Some(false)
        );
        assert_eq!(
            Requirement::Exact("nightly".to_owned()).holds("nightly"),
            Some(true)
        );
    }

    #[test]
    fn a_document_that_cannot_be_a_combination_is_refused_when_it_is_read() {
        let sandbox = sandbox();

        // Two members in one role: not a combination, a question.
        let path = sandbox.write(
            "two.json",
            &KIT.replace(
                "{\"role\": \"debugger\"",
                "{\"role\": \"toolchain\", \"name\": \"second\", \"version\": \"*\", \"serves\": \"*\", \"probe\": {\"kind\": \"variable\", \"variable\": \"RUSTC\"}}, {\"role\": \"debugger\"",
            ),
        );
        let error = Kit::read(&path).expect_err("two toolchains");
        assert!(error.message().contains("toolchain"), "{}", error.message());
        assert!(
            error
                .message()
                .contains("claims the same thing as an earlier entry"),
            "{}",
            error.message()
        );

        // A role this build does not know, a version it cannot read, and a version that says
        // nothing: three refusals, each saying which member is wrong.
        let unknown_role = sandbox.write(
            "role.json",
            &KIT.replace("\"role\": \"debugger\"", "\"role\": \"emulator\""),
        );
        assert!(Kit::read(&unknown_role).is_err());
        // A version requirement beside a path probe is refused: a path can say that something is
        // there and cannot say which version it is, and a requirement that can never be checked is
        // worse than no requirement at all.
        let path_with_version = sandbox.write(
            "path-version.json",
            &KIT.replace(
                "\"name\": \"linux-sysroot\", \"version\": \"*\"",
                "\"name\": \"linux-sysroot\", \"version\": \"1.0\"",
            ),
        );
        let error = Kit::read(&path_with_version).expect_err("a version beside a path");
        assert!(
            error
                .message()
                .contains("a path cannot say which version it is"),
            "{}",
            error.message()
        );

        // A version requirement that is not a dotted number is read as an exact string, and it is
        // then checked like any other: `latest` is not refused, it is a requirement this machine
        // will almost certainly not meet, and the diagnosis says which one it wanted.
        let named_version = sandbox.write(
            "version.json",
            &KIT.replace("\"version\": \">=1.98\"", "\"version\": \"latest\""),
        );
        let named = Kit::read(&named_version).expect("an exact string is a requirement");
        let diagnosis = named.diagnose(
            &Observed::nothing().with("rust", Found::Value("1.99.0".to_owned())),
            None,
        );
        assert!(matches!(
            diagnosis.findings()[0].finding(),
            super::Finding::Version { .. }
        ));
        assert!(
            diagnosis.findings()[0]
                .finding()
                .render()
                .contains("the kit asks for exactly `latest`"),
            "{}",
            diagnosis.findings()[0].finding().render()
        );
        let empty_version = sandbox.write(
            "empty.json",
            &KIT.replace(
                "\"role\": \"debugger\", \"name\": \"lldb\", \"version\": \"*\"",
                "\"role\": \"debugger\", \"name\": \"lldb\", \"version\": \"\"",
            ),
        );
        assert!(Kit::read(&empty_version).is_err());

        // Public fields are absent on purpose, so the assertions above go through accessors.
        assert!(MEMBER_MEMBERS.contains(&"serves"));
    }

    #[test]
    fn a_kit_is_not_an_output_and_a_profile_in_it_is_refused() {
        let sandbox = sandbox();
        for member in KIT_MEMBERS {
            assert!(
                !member.contains("profile") && !member.contains("triple"),
                "{member} should not be about the output"
            );
        }
        let with_profile = sandbox.write(
            "profile.json",
            &KIT.replace(
                "\"serves\": \"*\", \"probe\": {\"kind\": \"program\"",
                "\"serves\": \"*\", \"profile\": \"desktop\", \"probe\": {\"kind\": \"program\"",
            ),
        );
        let error = Kit::read(&with_profile).expect_err("a member about the output is refused");
        assert!(error.message().contains("`profile`"), "{}", error.message());
        assert!(
            error
                .message()
                .contains("belongs to the project's configuration"),
            "{}",
            error.message()
        );

        // The kit is asked about the one thing a tool has an opinion about: a triple.
        let kit = sandbox.kit(KIT);
        let none = kit.diagnose(&Observed::nothing(), None);
        assert_eq!(none.triple(), None);
        assert!(
            none.render()
                .join("\n")
                .contains("no output triple was asked about"),
            "{:?}",
            none.render()
        );
    }

    #[test]
    fn not_asked_a_missing_member_and_a_wrong_version_are_three_different_findings() {
        let sandbox = sandbox();
        let kit = sandbox.kit(KIT);

        // Nobody asked: not-checked, with the reason the record carries.
        let nothing = kit.diagnose(&Observed::nothing(), None);
        assert_eq!(nothing.problems().len(), 5);
        assert!(matches!(
            nothing.findings()[0].finding(),
            super::Finding::NotChecked { reason } if reason.contains("nobody asked")
        ));
        assert!(nothing.render().join("\n").contains("not checked"));

        // Somebody looked and found nothing: missing, and the diagnosis says what was found.
        let observed = Observed::nothing().with("rust", Found::Value("1.97.0".to_owned()));
        let old = kit.diagnose(&observed, None);
        assert!(matches!(
            old.findings()[0].finding(),
            super::Finding::Version { .. }
        ));
        assert!(
            old.render()
                .join("\n")
                .contains("found `1.97.0`, and the kit asks for `1.98` or newer"),
            "{:?}",
            old.render()
        );
        let missing = Observed::nothing().with(
            "rust",
            Found::Nothing {
                reason: "there is nothing at /opt/rust".to_owned(),
            },
        );
        let gone = kit.diagnose(&missing, None);
        assert!(matches!(
            gone.findings()[0].finding(),
            super::Finding::Missing { .. }
        ));
        assert!(
            gone.render()
                .join("\n")
                .contains("there is nothing at /opt/rust"),
            "{:?}",
            gone.render()
        );
        assert_eq!(gone.problems().len(), nothing.problems().len());
        assert_ne!(
            gone.findings()[0].finding(),
            nothing.findings()[0].finding()
        );
        assert_eq!(gone.findings()[0].finding().word(), "missing");
        assert_eq!(nothing.findings()[0].finding().word(), "not checked");

        // The version that satisfies it is `present`, and the whole kit is complete once every
        // member is found and matches.
        let mut all = Observed::nothing();
        for (member, value) in [
            ("rust", "1.99.0"),
            ("linux-sysroot", "1.0"),
            ("lldb", "18.1.8"),
            ("bench", "the bench machine"),
            ("RUSTUP_TOOLCHAIN", "1.99.0"),
        ] {
            all = all.with(member, Found::Value(value.to_owned()));
        }
        let complete = kit.diagnose(&all, Some("x86_64-unknown-linux-gnu"));
        assert!(complete.is_complete(), "{:?}", complete.render());
        assert!(
            complete
                .render()
                .join("\n")
                .contains("complete for this output")
        );
    }

    #[test]
    fn a_version_the_build_cannot_compare_is_not_checked_rather_than_decided() {
        let sandbox = sandbox();
        let kit = sandbox.kit(KIT);
        let observed = Observed::nothing().with("rust", Found::Value("1.99.0-nightly".to_owned()));

        let diagnosis = kit.diagnose(&observed, None);
        let finding = diagnosis.findings()[0].finding();
        assert!(matches!(finding, super::Finding::NotChecked { .. }));
        assert!(
            finding.render().contains("is not a dotted number"),
            "{}",
            finding.render()
        );
        assert!(!diagnosis.is_complete());
    }

    #[test]
    fn a_member_that_cannot_serve_the_output_is_a_finding_and_not_a_fallback() {
        let sandbox = sandbox();
        let kit = sandbox.kit(KIT);
        let mut observed = Observed::nothing();
        for member in ["rust", "linux-sysroot", "lldb", "bench", "RUSTUP_TOOLCHAIN"] {
            observed = observed.with(member, Found::Value("1.99.0".to_owned()));
        }

        let diagnosis = kit.diagnose(&observed, Some("aarch64-unknown-linux-gnu"));
        let sdk = diagnosis
            .findings()
            .iter()
            .find(|finding| finding.role() == Role::Sdk)
            .expect("the sdk");
        assert!(matches!(sdk.finding(), super::Finding::CannotServe { .. }));
        assert!(
            sdk.finding()
                .render()
                .contains("it cannot serve `aarch64-unknown-linux-gnu`"),
            "{}",
            sdk.finding().render()
        );
        assert_eq!(sdk.finding().word(), "cannot serve");
        let device = diagnosis
            .findings()
            .iter()
            .find(|finding| finding.role() == Role::Device)
            .expect("the device");
        assert!(matches!(
            device.finding(),
            super::Finding::CannotServe { .. }
        ));
        // The toolchain serves every triple, so it is still reported for what it is.
        let toolchain = &diagnosis.findings()[0];
        assert!(matches!(toolchain.finding(), super::Finding::Ok { .. }));
        assert!(!diagnosis.is_complete());
    }

    #[test]
    fn two_members_whose_triples_never_meet_are_an_incompatible_combination() {
        let sandbox = sandbox();
        let kit = sandbox.kit(&KIT.replace(
            "\"serves\": [\"x86_64-unknown-linux-gnu\"], \
            \"probe\": {\"kind\": \"path\", \"path\": \"sysroot\"}",
            "\"serves\": [\"aarch64-apple-darwin\"], \
            \"probe\": {\"kind\": \"path\", \"path\": \"sysroot\"}",
        ));
        let diagnosis = kit.diagnose(&Observed::nothing(), None);

        assert_eq!(
            diagnosis.incompatible().len(),
            2,
            "{:?}",
            diagnosis.render()
        );
        let said = diagnosis.incompatible().join(" ");
        assert!(said.contains("no output triple in common"), "{said}");
        assert!(said.contains("sdk linux-sysroot"), "{said}");
        assert!(!diagnosis.is_complete());
        // It is a statement about the combination, not about the output that was asked about: it is
        // reported even when nothing was asked about.
        assert!(diagnosis.triple().is_none());
        assert!(diagnosis.render().join("\n").contains("incompatible:"));
    }

    #[test]
    fn a_kit_without_a_toolchain_says_nothing_can_be_built() {
        let sandbox = sandbox();
        let kit = sandbox.kit(
            &KIT.replace(
                "{\"role\": \"toolchain\", \"name\": \"rust\", \"version\": \">=1.98\", \"serves\": \"*\", \
            \"probe\": {\"kind\": \"program\", \"command\": \"cargo\", \"argument\": \"--version\"}}, \
           ",
                "",
            ),
        );
        assert_eq!(kit.members().len(), 4);
        assert_eq!(kit.roles_without_a_member(), vec![Role::Toolchain]);
        let diagnosis = kit.diagnose(&Observed::nothing(), None);
        assert!(!diagnosis.is_complete());
        let rendered = diagnosis.render().join("\n");
        assert!(
            rendered.contains(
                "no member plays the toolchain role, and nothing can be built without one"
            ),
            "{rendered}"
        );
        // A role that is not required is reported the same way, without the sentence about nothing
        // being buildable: a kit for a task that needs no device is not an incomplete kit.
        assert!(Role::Toolchain.is_required());
        assert!(!Role::Device.is_required());
    }

    #[test]
    fn what_a_toolchain_check_found_is_translated_and_never_invented() {
        let report = ToolchainReport::from_checks(
            Ok(ToolVersion::from_line(
                "cargo",
                "cargo 1.99.0 (5f94df478 2026-08-27)",
            )),
            Err(ToolUnavailable::NotInstalled {
                program: "rustc".to_owned(),
                reason: "No such file or directory (os error 2)".to_owned(),
            }),
            vec!["cargo --version".to_owned(), "rustc --version".to_owned()],
        );
        let observed = Observed::from_toolchain(&ToolchainState::Checked(report));
        assert_eq!(observed.of("cargo"), Found::Value("1.99.0".to_owned()));
        // The translation keeps the check's own finding rather than writing its own sentence: the
        // reason a person reads here is the one the toolchain check produced.
        assert!(matches!(
            observed.of("rustc"),
            Found::Nothing { reason } if reason.contains("not installed")
        ));

        let unasked = Observed::from_toolchain(&ToolchainState::NotChecked(NotChecked::NotAsked));
        assert!(matches!(
            unasked.of("cargo"),
            Found::NotChecked { reason } if reason.contains("not checked yet")
        ));
        let refused = Observed::from_toolchain(&ToolchainState::NotChecked(NotChecked::Refused {
            reason: "the permission table said no".to_owned(),
        }));
        assert!(matches!(
            refused.of("cargo"),
            Found::NotChecked { reason } if reason.contains("permission table")
        ));
    }

    #[test]
    fn a_version_is_read_out_of_a_programs_own_output_by_one_stated_rule() {
        assert_eq!(
            version_like("cargo 1.99.0 (5f94df478 2026-08-27)"),
            Some("1.99.0".to_owned())
        );
        assert_eq!(
            version_like("lldb version 18.1.8 (https://lldb.llvm.org/)"),
            Some("18.1.8".to_owned())
        );
        // A date is not a version, and neither is a build hash or a bare number: the rule needs a
        // dot, and it needs the run to start where a version would.
        assert_eq!(version_like("built on 2026-08-27"), None);
        assert_eq!(version_like("no version here"), None);
        assert_eq!(version_like("5f94df478"), None);
        assert_eq!(version_like("1.99.0-nightly"), Some("1.99.0".to_owned()));
        assert_eq!(version_like("v1.2.3"), Some("1.2.3".to_owned()));
        assert_eq!(version_like(""), None);
    }

    #[test]
    fn a_probe_path_may_not_leave_the_directory_the_kit_is_in() {
        let sandbox = sandbox();
        let escaping = sandbox.write(
            "escaping.json",
            &KIT.replace("\"path\": \"sysroot\"", "\"path\": \"../somewhere/else\""),
        );
        let error = Kit::read(&escaping).expect_err("a probe that leaves the kit directory");
        assert!(
            error
                .message()
                .contains("does not stay inside the directory the kit is in"),
            "{}",
            error.message()
        );
        // A probe the kit cannot answer is not a document error: it is `undetermined`, and the
        // diagnosis says so.
        let undetermined = sandbox.kit(&KIT.replace(
            "{\"kind\": \"path\", \"path\": \"sysroot\"}",
            "{\"kind\": \"undetermined\", \"reason\": \"the sysroot is on another machine\"}",
        ));
        let diagnosis = undetermined.diagnose(&Observed::nothing(), None);
        let sdk = diagnosis
            .findings()
            .iter()
            .find(|finding| finding.role() == Role::Sdk)
            .expect("the sdk");
        assert!(matches!(
            sdk.finding(),
            super::Finding::NotChecked { reason } if reason.contains("nobody asked")
        ));
    }

    #[test]
    fn a_kit_document_round_trips_as_json() {
        let sandbox = sandbox();
        let kit = sandbox.kit(KIT);
        let text = kit.to_json().render();
        for member in KIT_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        for member in super::MEMBER_MEMBERS {
            assert!(text.contains(&format!("\"{member}\"")), "{member}: {text}");
        }
        // What it writes, it reads: the members, their versions and their probes survive.
        let again = sandbox.write("again.json", &text);
        let reparsed = Kit::read(&again).expect("what was written reads back");
        assert_eq!(reparsed.members().len(), kit.members().len());
        assert_eq!(reparsed.to_json().render(), text);
        assert_eq!(
            reparsed
                .member(Role::Toolchain)
                .expect("a member")
                .probe()
                .render(),
            "`cargo --version`"
        );
        assert_eq!(
            reparsed
                .member(Role::Sdk)
                .expect("a member")
                .serves()
                .render(),
            "x86_64-unknown-linux-gnu"
        );
    }
}
