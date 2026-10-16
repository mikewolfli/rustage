// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The projects that were opened before: places, never permissions (task T02.03).
//!
//! A recent list answers one question — *where was that project?* — and it must answer nothing else.
//! The acceptance line is "a recent entry does not trigger execution", and this module makes that
//! true by what it contains rather than by what it avoids: it holds paths, names, identities and the
//! time they were opened, it has no permission service and no way to start a program, and the only
//! thing a caller can do with an entry is [`crate::project::lifecycle::session::ProjectSession::from_recent`],
//! which opens the project exactly as if the path had been typed by hand — untrusted, read-only
//! until a person decides, running nothing.
//!
//! **Repair is by identity, not by path.** A project that moved is the same project only if the
//! document in the new place carries the identity that was recorded; anything else is a different
//! project, and quietly re-pointing an entry at it would be how a person ends up opening something
//! they never chose. So [`RecentProjects::repair`] refuses a different project, refuses to repair an
//! entry whose path is still there, and refuses a place it cannot read.
//!
//! The list is machine-local state, which is why it does not live in a project document
//! (`crate::project`): it belongs to this machine's configuration directory, and the file is written
//! the same way a project document is — a temporary file first, then a rename.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::contracts::revision::FormatVersion;
use crate::project::record::format::{ProjectError, format_version, unix_millis};
use crate::project::record::json::{self, Json, JsonKind, JsonObject, Number};

/// The recent list format this build writes.
///
/// Its own sequence, like the page document's: this file is machine-local state, and it must not
/// change meaning because a project document gained a member.
pub const RECENT_FORMAT: FormatVersion = FormatVersion::from_raw(1);

/// The member names a recent list version 1 defines, in the order the writer writes them.
pub const RECENT_DOCUMENT_MEMBERS: [&str; 3] = ["format", "recent", "absent"];

/// The member names an entry version 1 defines, in the order the writer writes them.
pub const RECENT_ENTRY_MEMBERS: [&str; 5] = [
    "path",
    "name",
    "id",
    "opened_at_unix_ms",
    "missing_since_unix_ms",
];

/// One project that was opened before.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecentEntry {
    path: PathBuf,
    name: String,
    id: String,
    opened_at_millis: i64,
    missing_since: Option<i64>,
}

impl RecentEntry {
    /// An entry for a project that was opened at a time.
    pub fn new(
        path: impl Into<PathBuf>,
        name: impl Into<String>,
        id: impl Into<String>,
        at: SystemTime,
    ) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            id: id.into(),
            opened_at_millis: unix_millis(at).unwrap_or_default(),
            missing_since: None,
        }
    }

    /// Where the project was.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What it was called when it was opened.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Which project it was, as its own document said.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// When it was last opened, in whole milliseconds since 1970.
    pub const fn opened_at_millis(&self) -> i64 {
        self.opened_at_millis
    }

    /// When it was noticed to be gone, when it was.
    ///
    /// Remembering this is what lets a list say "gone since Tuesday" instead of silently dropping an
    /// entry somebody is looking for.
    pub const fn missing_since_millis(&self) -> Option<i64> {
        self.missing_since
    }

    fn mark_missing(&mut self, at_millis: i64) {
        self.missing_since = Some(at_millis);
    }

    fn mark_present(&mut self) {
        self.missing_since = None;
    }
}

/// Whether a recorded place still holds what it held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecentState {
    /// The directory is there.
    Present,
    /// The directory is not there any more; the entry can be repaired or forgotten.
    Missing,
}

impl RecentState {
    /// The word a report uses.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Missing => "missing",
        }
    }
}

/// Why an entry could not be repaired.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RepairRefusal {
    /// No entry names that path.
    NoSuchEntry {
        /// The path.
        path: PathBuf,
    },
    /// The recorded path is still there, so there is nothing to repair.
    NotMissing {
        /// The path.
        path: PathBuf,
    },
    /// The place the entry should point at is not a project this build can read.
    Unreadable(ProjectError),
    /// The place holds a different project than the one that was recorded.
    DifferentProject {
        /// The place.
        path: PathBuf,
        /// The identity that was recorded.
        recorded: String,
        /// The identity that is there now.
        found: String,
    },
}

impl RepairRefusal {
    /// A sentence naming what was found.
    pub fn message(&self) -> String {
        match self {
            Self::NoSuchEntry { path } => {
                format!("no recent entry names {}", path.display())
            }
            Self::NotMissing { path } => format!(
                "{} is still there, so this entry does not need repairing",
                path.display()
            ),
            Self::Unreadable(error) => error.message(),
            Self::DifferentProject {
                path,
                recorded,
                found,
            } => format!(
                "{} holds a different project (recorded {recorded}, found {found}), so the entry \
                 was not moved onto it",
                path.display()
            ),
        }
    }
}

impl std::fmt::Display for RepairRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message())
    }
}

/// The projects that were opened before, most recent first.
#[derive(Clone, Debug)]
pub struct RecentProjects {
    path: PathBuf,
    entries: Vec<RecentEntry>,
    capacity: usize,
}

impl RecentProjects {
    /// How many entries a list keeps. Older ones fall off the end.
    pub const CAPACITY: usize = 10;

    /// An empty list that would be saved at a path.
    pub fn empty(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            entries: Vec::new(),
            capacity: Self::CAPACITY,
        }
    }

    /// A list with a different bound, which is what a test uses.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self.entries.truncate(self.capacity);
        self
    }

    /// Reads the list, or an empty one when the file is not there yet.
    ///
    /// A file that is there and cannot be read is an error rather than an empty list: quietly
    /// forgetting every recent project because one file went wrong is a loss a person would not see.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let path = path.as_ref().to_path_buf();
        let mut list = Self::empty(path.clone());
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(list),
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
        match version.get().cmp(&RECENT_FORMAT.get()) {
            std::cmp::Ordering::Equal => {}
            std::cmp::Ordering::Greater => {
                return Err(ProjectError::NewerFormat {
                    found: version,
                    current: RECENT_FORMAT,
                });
            }
            std::cmp::Ordering::Less => {
                return Err(ProjectError::OlderFormat {
                    found: version,
                    current: RECENT_FORMAT,
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
        let Some(recent) = object.get("recent") else {
            return Err(ProjectError::MissingMember {
                path: "recent".to_owned(),
            });
        };
        let Some(values) = recent.as_array() else {
            return Err(ProjectError::WrongType {
                path: "recent".to_owned(),
                expected: "an array",
                found: recent.kind(),
            });
        };
        for (index, value) in values.iter().enumerate() {
            list.entries.push(decode_entry(index, value)?);
        }
        list.entries.truncate(list.capacity);
        Ok(list)
    }

    /// Writes the list, through a temporary file and a rename.
    pub fn save(&self) -> Result<PathBuf, ProjectError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| ProjectError::Io {
                path: parent.to_path_buf(),
                action: "creating the directory the recent list lives in",
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

    /// Where the list is kept.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The entries, most recent first.
    pub fn entries(&self) -> &[RecentEntry] {
        &self.entries
    }

    /// How many entries there are.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records that a project was opened, and returns its position.
    ///
    /// One entry per path: opening a project again moves it to the front rather than adding a second
    /// line for the same place. Recording is all this does — nothing here is trusted, authorized or
    /// run, and this module has no way to do any of those.
    pub fn record(&mut self, entry: RecentEntry) -> usize {
        let position = self
            .entries
            .iter()
            .position(|known| known.path == entry.path)
            .unwrap_or(usize::MAX);
        let mut entry = entry;
        if position == usize::MAX {
            self.entries.insert(0, entry);
        } else {
            entry.mark_present();
            self.entries.remove(position);
            self.entries.insert(0, entry);
        }
        self.entries.truncate(self.capacity);
        0
    }

    /// Whether an entry's place still holds something, and remembers it when it does not.
    pub fn state_of(&mut self, entry: &RecentEntry) -> RecentState {
        let path = entry.path.clone();
        let present = path.is_dir();
        let now = unix_millis(SystemTime::now()).unwrap_or_default();
        for known in &mut self.entries {
            if known.path != path {
                continue;
            }
            if present {
                known.mark_present();
            } else if known.missing_since.is_none() {
                known.mark_missing(now);
            }
        }
        if present {
            RecentState::Present
        } else {
            RecentState::Missing
        }
    }

    /// Forgets the entry for a path.
    pub fn forget(&mut self, path: &Path) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.path != path);
        self.entries.len() != before
    }

    /// Points an entry at where its project is now, which is only allowed for the same project.
    ///
    /// The identity is what makes it the same project: the document in the new place has to carry the
    /// identity that was recorded. An entry whose path is still there is not repaired (that would be
    /// a way to change what an entry points at without the project having moved), and a place that
    /// holds a different project is refused with both identities named.
    pub fn repair(&mut self, from: &Path, to: &Path) -> Result<(), RepairRefusal> {
        let Some(position) = self.entries.iter().position(|entry| entry.path == from) else {
            return Err(RepairRefusal::NoSuchEntry {
                path: from.to_path_buf(),
            });
        };
        if from.is_dir() {
            return Err(RepairRefusal::NotMissing {
                path: from.to_path_buf(),
            });
        }
        if !to.is_dir() {
            return Err(RepairRefusal::Unreadable(ProjectError::Io {
                path: to.to_path_buf(),
                action: "reading the project the entry should point at",
                reason: "it is not a directory".to_owned(),
            }));
        }
        // The entry remembers a directory, so the document is the one the format puts in it.
        let document = to.join(crate::project::record::format::PROJECT_FILE_NAME);
        let session = crate::project::lifecycle::session::ProjectSession::open(&document)
            .map_err(RepairRefusal::Unreadable)?;
        let found = session
            .id()
            .map(str::to_owned)
            .unwrap_or_else(|| "<no identity>".to_owned());
        let recorded = self.entries[position].id.clone();
        if found != recorded {
            return Err(RepairRefusal::DifferentProject {
                path: to.to_path_buf(),
                recorded,
                found,
            });
        }
        let entry = &mut self.entries[position];
        entry.path = to.to_path_buf();
        entry.mark_present();
        Ok(())
    }

    /// The list as JSON.
    pub fn to_json(&self) -> Json {
        let mut out = JsonObject::new();
        out.insert("format", Json::number(u64::from(RECENT_FORMAT.get())));
        out.insert(
            "recent",
            Json::array(self.entries.iter().map(encode_entry).collect::<Vec<_>>()),
        );
        out.insert("absent", Json::array(Vec::new()));
        Json::Object(out)
    }

    /// One line per entry, for a report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.entries.is_empty() {
            lines.push("no projects have been opened yet".to_owned());
            return lines;
        }
        for entry in &self.entries {
            let state = if entry.path.is_dir() {
                "present"
            } else {
                "missing"
            };
            lines.push(format!(
                "{state:<8} {} — {} ({})",
                entry.path.display(),
                entry.name,
                entry.id
            ));
        }
        lines
    }
}

fn encode_entry(entry: &RecentEntry) -> Json {
    let mut out = JsonObject::new();
    out.insert(
        "path",
        Json::string(entry.path.to_string_lossy().into_owned()),
    );
    out.insert("name", Json::string(&entry.name));
    out.insert("id", Json::string(&entry.id));
    out.insert(
        "opened_at_unix_ms",
        Json::Number(Number::from_i64(entry.opened_at_millis)),
    );
    out.insert(
        "missing_since_unix_ms",
        match entry.missing_since {
            Some(millis) => Json::Number(Number::from_i64(millis)),
            None => Json::Null,
        },
    );
    Json::Object(out)
}

fn decode_entry(index: usize, value: &Json) -> Result<RecentEntry, ProjectError> {
    let Some(object) = value.as_object() else {
        return Err(ProjectError::WrongType {
            path: format!("recent[{index}]"),
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
                path: format!("recent[{index}].{name}"),
                expected: "a string",
                found: object.get(name).map(Json::kind).unwrap_or(JsonKind::Null),
            })
    };
    let millis = |name: &str| -> Option<i64> {
        object
            .get(name)
            .and_then(Json::as_i64)
            .filter(|_| object.get(name).map(Json::kind) == Some(JsonKind::Number))
    };
    Ok(RecentEntry {
        path: PathBuf::from(text("path")?),
        name: text("name")?,
        id: text("id")?,
        opened_at_millis: millis("opened_at_unix_ms").unwrap_or_default(),
        missing_since: millis("missing_since_unix_ms"),
    })
}
