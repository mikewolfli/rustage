// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The argument parser and the usage text.
//!
//! The surface is deliberately tiny. The scaffold answers `--help`, `--version` and reports its
//! own status; the other two options exist because the host needs reproducible producers for
//! artifacts it commits or inspects: `--render-icons` renders the icon strip in
//! [`crate::app::toolbar`] into a directory (the committed baselines under `tests/baselines/icons/`
//! would otherwise be pictures of whatever the machine last rendered, which is exactly what a
//! baseline must not be), and `--probe-apis` runs the dependency contract probes in
//! [`crate::probe`] and writes their report.

use std::{
    ffi::{OsStr, OsString},
    fmt,
    path::PathBuf,
    time::Duration,
};

use crate::host::log::Level;
use crate::project::{
    configuration::SettingValue,
    create::Template,
    files::FileOperation,
    workspace::{WorkspaceFile, name_is_addressable},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Status,
    /// Writes the icon strip's light and dark renders into the given directory.
    RenderIcons(PathBuf),
    /// Runs the dependency contract probes and writes the report into the given directory.
    ProbeApis(PathBuf),
    /// Runs the host capability checks and writes the report into the given directory.
    ProbeHost(PathBuf),
    /// Reads a project file, reports what this build makes of it, and writes the round trip next to
    /// it as evidence that nothing in it was dropped.
    ProjectCheck(PathBuf),
    /// Creates a project: the new-project wizard's work, without a window (task T02.02).
    NewProject(NewProjectOptions),
    /// Opens a project: what it is, what would be asked about it, and nothing else (task T02.03).
    OpenProject(OpenProjectOptions),
    /// Lists the projects that were opened before, and where they are now.
    Recent,
    /// Shows what a project has, classified the way a project tree groups it (task T02.04).
    ProjectTree(PathBuf),
    /// Removes an entry, fixing what points at it or refusing to remove it (task T02.04).
    DropEntry(DropEntryOptions),
    /// Previews or performs one file operation on a project (task T02.05).
    FileOp(FileOpOptions),
    /// Shows the build configuration of a project, and what each value came from (task T02.06).
    Configuration(ConfigurationOptions),
    /// Imports an existing project: what it is, what would change, and the migration itself
    /// (task T02.07).
    Import(ImportOptions),
    /// Checks what a project needs, re-scans it, and changes nothing (task T02.08).
    Validate(ValidateOptions),
    /// Shows a workspace, and plans or carries out one change to its members (task T02.09).
    Workspace(WorkspaceOptions),
    /// Creates an empty workspace document in a directory (task T02.09).
    NewWorkspace(NewWorkspaceOptions),
    /// Shows which tools a kit is made of, and what this machine has of them (task T02.10).
    Kit(KitOptions),
    /// Starts the baseline desktop session and leaves it again.
    Launch(LaunchOptions),
}

/// What to create a project from.
///
/// The wizard's steps are its arguments: where the project goes, what it is called, and what it
/// starts from. A window would collect the same three things, and the creation path below it is the
/// same one either way.
#[derive(Debug, PartialEq, Eq)]
pub struct NewProjectOptions {
    /// The directory the project is created in.
    pub directory: PathBuf,
    /// The name a person sees; the directory's own name when nothing is given.
    pub name: Option<String>,
    /// What the project starts from.
    pub template: Template,
}

impl NewProjectOptions {
    /// Options for a directory, starting from a template.
    pub fn new(directory: impl Into<PathBuf>, template: Template) -> Self {
        Self {
            directory: directory.into(),
            name: None,
            template,
        }
    }
}

/// How a project should be opened.
///
/// `trust` is the answer to the question the open prints, and `diagnose` asks for the toolchain
/// check, which is only possible once the project is trusted. Neither is implied by the other: a
/// person can open a project and read it without deciding anything.
#[derive(Debug, PartialEq, Eq)]
pub struct OpenProjectOptions {
    /// The project document to open.
    pub path: PathBuf,
    /// Whether the person answering the question said yes.
    pub trust: bool,
    /// Whether to read the toolchain, which runs two programs.
    pub diagnose: bool,
}

impl OpenProjectOptions {
    /// Options that read a project and decide nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            trust: false,
            diagnose: false,
        }
    }
}

/// Which entry to remove, and how to treat what points at it.
#[derive(Debug, PartialEq, Eq)]
pub struct DropEntryOptions {
    /// The project document.
    pub path: PathBuf,
    /// The page document of the entry, as the project writes it.
    pub document: String,
    /// Whether the references to it may be fixed, instead of refusing the removal.
    pub fix: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
}

impl DropEntryOptions {
    /// Options that remove an entry and refuse while anything points at it.
    pub fn new(path: impl Into<PathBuf>, document: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            document: document.into(),
            fix: false,
            trust: false,
        }
    }
}

/// Which existing project to import, and how far to go.
#[derive(Debug, PartialEq, Eq)]
pub struct ImportOptions {
    /// The project document, or the directory that holds one.
    pub path: PathBuf,
    /// Whether the migration should be carried out instead of only described.
    pub apply: bool,
    /// Whether to report the files a past migration kept, instead of planning a new one.
    pub check_backup: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
}

impl ImportOptions {
    /// Options that describe an import and write nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            apply: false,
            check_backup: false,
            trust: false,
        }
    }
}

/// How a kit should be diagnosed (task T02.10).
///
/// Showing a kit runs nothing. `diagnose` asks this machine about each member — which runs the
/// programs the kit names, so it needs the directory to be trusted and the permission the decision
/// table gives a command — and `triple` is the output the kit is asked about. Which profile is being
/// built is not part of the question: that belongs to the project's configuration, and a kit is a
/// combination of tools.
#[derive(Debug, PartialEq, Eq)]
pub struct KitOptions {
    /// The kit document.
    pub path: PathBuf,
    /// Whether to ask this machine about each member.
    pub diagnose: bool,
    /// The output triple to ask about, when there is one.
    pub triple: Option<String>,
    /// Whether the person answering the directory's question said yes.
    pub trust: bool,
}

impl KitOptions {
    /// Options that show a kit and ask nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            diagnose: false,
            triple: None,
            trust: false,
        }
    }
}

/// What to do with a workspace.
#[derive(Debug, PartialEq, Eq)]
pub enum WorkspaceAction {
    /// Show it: its members, what each holds, and its files qualified by member.
    Report,
    /// Name a root that is already there as a member.
    Add {
        /// The name the member is addressed by.
        name: String,
        /// The root, relative to the workspace root.
        root: String,
    },
    /// Take a member out of the document, and touch no directory.
    Remove {
        /// The member's name.
        name: String,
    },
    /// Record which member is active, on this machine.
    Activate {
        /// The member's name.
        name: String,
    },
    /// Show one file, addressed as `member:path`.
    File {
        /// The `member:path` name.
        name: String,
    },
}

/// How a workspace should be looked at, and how far to go (task T02.09).
///
/// Showing a workspace writes nothing; a change is planned first and only carried out with `apply`,
/// which is what asks the directory's question and takes the write permission. What a change writes
/// is the workspace document and nothing else: a member is a line in it, so adding names a root that
/// is already there and removing leaves the directory exactly as it is.
#[derive(Debug, PartialEq, Eq)]
pub struct WorkspaceOptions {
    /// The workspace document.
    pub path: PathBuf,
    /// What to do.
    pub action: WorkspaceAction,
    /// Whether a planned change should be carried out.
    pub apply: bool,
    /// Whether the person answering the directory's question said yes.
    pub trust: bool,
}

impl WorkspaceOptions {
    /// Options that show a workspace and change nothing.
    pub fn new(path: impl Into<PathBuf>, action: WorkspaceAction) -> Self {
        Self {
            path: path.into(),
            action,
            apply: false,
            trust: false,
        }
    }
}

/// What to create a workspace from.
#[derive(Debug, PartialEq, Eq)]
pub struct NewWorkspaceOptions {
    /// The directory the workspace document goes in, which has to be there.
    pub directory: PathBuf,
    /// The name a person sees; the directory's own name when nothing is given.
    pub name: Option<String>,
}

impl NewWorkspaceOptions {
    /// Options for a directory.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            name: None,
        }
    }
}

/// How a project should be checked, and what should be recorded or compared (task T02.08).
///
/// Checking writes nothing outside the two paths below, and never the project document: `record` is
/// where a scan of the project is written, and `rescan` is the scan to compare against. Both are
/// optional, and neither is the project.
#[derive(Debug, PartialEq, Eq)]
pub struct ValidateOptions {
    /// The project document.
    pub path: PathBuf,
    /// Whether to read the toolchain, which runs two programs.
    pub diagnose: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
    /// Where to write a scan of the project, when one should be kept.
    pub record: Option<PathBuf>,
    /// The scan to compare against, when a re-scan was asked for.
    pub rescan: Option<PathBuf>,
}

impl ValidateOptions {
    /// Options that check a project and write nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            diagnose: false,
            trust: false,
            record: None,
            rescan: None,
        }
    }
}

/// An override given on the command line: which setting, and what value.
#[derive(Debug, PartialEq, Eq)]
pub struct OverrideWord {
    /// The setting's word.
    pub key: String,
    /// The value, as a word or as a list.
    pub value: SettingValue,
}

/// How a project's configuration should be shown, and whether anything should be recorded.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigurationOptions {
    /// The project document.
    pub path: PathBuf,
    /// The target to resolve for, instead of the one this machine chose.
    pub target: Option<String>,
    /// Overrides for this run, which are also what `--remember` records.
    pub overrides: Vec<OverrideWord>,
    /// Whether the target and the overrides should be recorded for this project on this machine.
    pub remember: bool,
    /// Whether everything recorded for this project should be forgotten.
    pub forget: bool,
}

impl ConfigurationOptions {
    /// Options that show a project's configuration and record nothing.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            target: None,
            overrides: Vec::new(),
            remember: false,
            forget: false,
        }
    }
}

/// Which file operation to plan, and whether to do it.
///
/// Without `apply` this only reads the project and prints what the operation would do, which is
/// what the design surface shows before anyone agrees to it. With `apply` the plan is carried out:
/// the files move first, then the document is written, and a failed write puts the moves back.
#[derive(Debug, PartialEq, Eq)]
pub struct FileOpOptions {
    /// The project document.
    pub path: PathBuf,
    /// The operation, as given on the command line.
    pub operation: FileOperation,
    /// Whether the plan should be carried out instead of only printed.
    pub apply: bool,
    /// Whether the person answering the project's question said yes.
    pub trust: bool,
}

impl FileOpOptions {
    /// Options that preview an operation on a project.
    pub fn new(path: impl Into<PathBuf>, operation: FileOperation) -> Self {
        Self {
            path: path.into(),
            operation,
            apply: false,
            trust: false,
        }
    }
}

/// How long the baseline session should live.
///
/// A start that cannot end is not evidence and not usable by a person, so the duration is part of
/// the command rather than a hidden constant; the default matches
/// [`crate::host::platform::BASELINE_SESSION`].
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchOptions {
    pub exit_after: Duration,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            exit_after: crate::host::platform::BASELINE_SESSION,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    argument: OsString,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unexpected argument `{}`",
            self.argument.to_string_lossy()
        )
    }
}

impl std::error::Error for ParseError {}

/// Reads the global `--log-level <level>` option wherever it appears.
///
/// It is global rather than per-command because the logger is installed once for the process: a
/// `--launch` that logs and a `--probe-apis` that logs must not each invent their own switch.
pub fn log_level_of<I, S>(arguments: I) -> Result<Option<Level>, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let arguments: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut level = None;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == OsStr::new("--log-level") {
            let Some(value) = arguments.get(index + 1) else {
                return Err(ParseError {
                    argument: arguments[index].clone(),
                });
            };
            let Some(parsed) = value.to_str().and_then(Level::parse) else {
                return Err(ParseError {
                    argument: value.clone(),
                });
            };
            level = Some(parsed);
            index += 2;
            continue;
        }
        index += 1;
    }
    Ok(level)
}

pub fn parse<I, S>(arguments: I) -> Result<Command, ParseError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // The global option is removed here so every arm below can assume it is looking at the command;
    // `log_level_of` reads it separately, and both agree because they share this token handling.
    let collected: Vec<OsString> = arguments
        .into_iter()
        .map(|value| value.as_ref().to_os_string())
        .collect();
    let mut stripped: Vec<OsString> = Vec::with_capacity(collected.len());
    let mut index = 0;
    while index < collected.len() {
        if collected[index] == OsStr::new("--log-level") {
            let Some(value) = collected.get(index + 1) else {
                return Err(ParseError {
                    argument: collected[index].clone(),
                });
            };
            if value.to_str().and_then(Level::parse).is_none() {
                return Err(ParseError {
                    argument: value.clone(),
                });
            }
            index += 2;
            continue;
        }
        stripped.push(collected[index].clone());
        index += 1;
    }

    let mut arguments = stripped.into_iter();
    let Some(argument) = arguments.next() else {
        return Ok(Command::Status);
    };
    let text = argument.to_str().unwrap_or_default();

    let command = match text {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        "--launch" => {
            let mut options = LaunchOptions::default();
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                if argument == OsStr::new("--exit-after-ms") {
                    let Some(value) = rest.next() else {
                        return Err(ParseError { argument });
                    };
                    let Some(milliseconds) =
                        value.to_str().and_then(|value| value.parse::<u64>().ok())
                    else {
                        return Err(ParseError { argument: value });
                    };
                    options.exit_after = Duration::from_millis(milliseconds);
                    continue;
                }
                return Err(ParseError { argument });
            }
            return Ok(Command::Launch(options));
        }
        "--new-project" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = NewProjectOptions::new(PathBuf::from(directory), Template::BasicPage);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--template" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(template) = value.to_str().and_then(Template::from_name) else {
                            return Err(ParseError { argument: value });
                        };
                        options.template = template;
                    }
                    "--name" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.name = Some(name.to_owned());
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::NewProject(options));
        }
        "--open-project" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = OpenProjectOptions::new(PathBuf::from(path));
            for argument in arguments {
                match argument.to_str().unwrap_or_default() {
                    "--trust" => options.trust = true,
                    "--diagnose" => options.diagnose = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::OpenProject(options));
        }
        "--recent" => {
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            return Ok(Command::Recent);
        }
        "--project-tree" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            return Ok(Command::ProjectTree(PathBuf::from(path)));
        }
        "--drop-entry" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = DropEntryOptions::new(PathBuf::from(path), "");
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--document" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(document) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.document = document.to_owned();
                    }
                    "--fix" => options.fix = true,
                    "--trust" => options.trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            if options.document.is_empty() {
                return Err(ParseError { argument });
            }
            return Ok(Command::DropEntry(options));
        }
        "--file-op" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            let mut operation = None;
            let mut apply = false;
            let mut trust = false;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--add" | "--exclude" | "--delete" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        operation = Some(match argument.to_str().unwrap_or_default() {
                            "--add" => FileOperation::Add {
                                path: path.to_owned(),
                            },
                            "--exclude" => FileOperation::Exclude {
                                path: path.to_owned(),
                            },
                            _ => FileOperation::Delete {
                                path: path.to_owned(),
                            },
                        });
                    }
                    "--move" | "--rename" => {
                        let Some(from) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(to) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let (Some(from), Some(to)) = (from.to_str(), to.to_str()) else {
                            return Err(ParseError { argument });
                        };
                        operation = Some(FileOperation::Move {
                            from: from.to_owned(),
                            to: to.to_owned(),
                        });
                    }
                    "--apply" => apply = true,
                    "--trust" => trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            let Some(operation) = operation else {
                return Err(ParseError { argument });
            };
            let mut options = FileOpOptions::new(PathBuf::from(path), operation);
            options.apply = apply;
            options.trust = trust;
            return Ok(Command::FileOp(options));
        }
        "--kit" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = KitOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--diagnose" => options.diagnose = true,
                    "--trust" => options.trust = true,
                    "--triple" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(triple) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if triple.trim().is_empty() {
                            return Err(ParseError { argument: value });
                        }
                        options.triple = Some(triple.trim().to_owned());
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Kit(options));
        }
        "--new-workspace" => {
            let Some(directory) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = NewWorkspaceOptions::new(PathBuf::from(directory));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--name" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.name = Some(name.to_owned());
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::NewWorkspace(options));
        }
        "--workspace" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = WorkspaceOptions::new(PathBuf::from(path), WorkspaceAction::Report);
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--add" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(pair) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((name, root)) = pair.split_once('=') else {
                            return Err(ParseError { argument: value });
                        };
                        if !name_is_addressable(name) || root.trim().is_empty() {
                            return Err(ParseError { argument: value });
                        }
                        options.action = WorkspaceAction::Add {
                            name: name.to_owned(),
                            root: root.to_owned(),
                        };
                    }
                    "--remove" | "--activate" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if !name_is_addressable(name) {
                            return Err(ParseError { argument: value });
                        }
                        options.action = if flag == "--remove" {
                            WorkspaceAction::Remove {
                                name: name.to_owned(),
                            }
                        } else {
                            WorkspaceAction::Activate {
                                name: name.to_owned(),
                            }
                        };
                    }
                    "--file" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(name) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if WorkspaceFile::parse(name).is_err() {
                            return Err(ParseError { argument: value });
                        }
                        options.action = WorkspaceAction::File {
                            name: name.to_owned(),
                        };
                    }
                    "--apply" => options.apply = true,
                    "--trust" => options.trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Workspace(options));
        }
        "--validate" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ValidateOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--diagnose" => options.diagnose = true,
                    "--trust" => options.trust = true,
                    "--record" | "--rescan" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(path) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        if argument.to_str().unwrap_or_default() == "--record" {
                            options.record = Some(PathBuf::from(path));
                        } else {
                            options.rescan = Some(PathBuf::from(path));
                        }
                    }
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Validate(options));
        }
        "--configuration" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ConfigurationOptions::new(PathBuf::from(path));
            let mut rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--target" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(target) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        options.target = Some(target.trim().to_owned());
                    }
                    "--override" => {
                        let Some(value) = rest.next() else {
                            return Err(ParseError { argument });
                        };
                        let Some(word) = value.to_str() else {
                            return Err(ParseError { argument: value });
                        };
                        let Some((key, value)) = word.split_once('=') else {
                            return Err(ParseError { argument: value });
                        };
                        if key.trim().is_empty() {
                            return Err(ParseError {
                                argument: word.into(),
                            });
                        }
                        // One spelling on the command line, two shapes in the model: a setting that
                        // holds a list takes a comma-separated one, and an empty word means the empty
                        // list rather than a missing value.
                        let value = if key.trim() == "features" || value.contains(',') {
                            SettingValue::list(value.split(','))
                        } else {
                            SettingValue::Word(value.trim().to_owned())
                        };
                        options.overrides.push(OverrideWord {
                            key: key.trim().to_owned(),
                            value,
                        });
                    }
                    "--remember" => options.remember = true,
                    "--forget" => options.forget = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            return Ok(Command::Configuration(options));
        }
        "--import" => {
            let Some(path) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut options = ImportOptions::new(PathBuf::from(path));
            for argument in arguments {
                match argument.to_str().unwrap_or_default() {
                    "--apply" => options.apply = true,
                    "--check-backup" => options.check_backup = true,
                    "--trust" => options.trust = true,
                    _ => return Err(ParseError { argument }),
                }
            }
            if options.apply && options.check_backup {
                return Err(ParseError { argument });
            }
            return Ok(Command::Import(options));
        }
        "--render-icons" | "--probe-apis" | "--probe-host" | "--project-check" => {
            let Some(value) = arguments.next() else {
                return Err(ParseError { argument });
            };
            let mut rest = arguments;
            if let Some(argument) = rest.next() {
                return Err(ParseError { argument });
            }
            let path = PathBuf::from(value);
            return Ok(match text {
                "--render-icons" => Command::RenderIcons(path),
                "--probe-apis" => Command::ProbeApis(path),
                "--project-check" => Command::ProjectCheck(path),
                _ => Command::ProbeHost(path),
            });
        }
        _ => return Err(ParseError { argument }),
    };

    if let Some(argument) = arguments.next() {
        return Err(ParseError { argument });
    }

    Ok(command)
}

pub fn help_text() -> &'static str {
    "rustage - Rust desktop UI designer\n\n\
     Usage: rustage [--help | --version | --launch [--exit-after-ms <ms>]]\n\
            rustage [--render-icons <directory> | --probe-apis <directory>]\n\
            rustage [--probe-host <directory> | --project-check <file>]\n\
            rustage --new-project <directory> [--template <name>] [--name <name>]\n\
            rustage [--open-project <file> [--trust] [--diagnose] | --recent]\n\
            rustage [--project-tree <file>]\n\
            rustage --drop-entry <file> --document <path> [--fix] [--trust]\n\
            rustage --file-op <file> [--add <path> | --exclude <path> | --delete <path>]\n\
                                 [--move <from> <to>] [--apply] [--trust]\n\n\
            rustage --configuration <file> [--target <name>] [--override <key>=<value>]\n\
                                  [--remember | --forget]\n\n\
            rustage --import <file-or-directory> [--apply | --check-backup] [--trust]\n\
            rustage --validate <file> [--diagnose] [--trust]\n\
                                 [--record <file> | --rescan <file>]\n\
            rustage --kit <file> [--diagnose] [--trust] [--triple <triple>]\n\
            rustage --new-workspace <directory> [--name <name>]\n\
            rustage --workspace <file> [--add <name>=<root> | --remove <name> |\n\
                                       --activate <name> | --file <member>:<path>]\n\
                                      [--apply] [--trust]\n\n\
     With no options, prints the current project status.\n\n\
     Options:\n\
       -h, --help                    Print this help message\n\
       -V, --version                 Print the application version\n\
       --launch                      Start the baseline desktop session, then leave it again\n\
       --exit-after-ms <ms>          With --launch, leave the event loop after <ms>\n\
       --render-icons <directory>    Render the icon strip, light and dark, into <directory>\n\
       --probe-apis <directory>      Run the dependency contract probes and write api-probes.json\n\
       --probe-host <directory>      Check the host platform services and write host-readiness.json\n\
       --project-check <file>        Read a project file, report what is understood and what is\n\
                                     kept, and write <file>.roundtrip.json as evidence\n\
       --new-project <directory>     Create a project; --template empty|basic-page|application\n\
                                     (default: basic-page), --name <name> (default: the directory)\n\
       --open-project <file>         Open a project read-only, and print what would be asked\n\
       --trust                       With --open-project, answer the question with yes\n\
       --diagnose                    With --open-project, read the toolchain (runs two programs)\n\
       --recent                      List the projects opened before, and where they are now\n\
       --project-tree <file>         Show what the project has, classified, with what is on disk\n\
       --drop-entry <file>           Remove the entry in --document; refuses while anything points\n\
                                     at it unless --fix is given, which fixes those references\n\
       --document <path>             With --drop-entry, the page document of the entry\n\
       --fix                         With --drop-entry, fix the references instead of refusing\n\
       --file-op <file>              Plan one file operation on the project; --add, --exclude,\n\
                                     --delete <path> or --move <from> <to> says which. Prints\n\
                                     what it would do unless --apply is given, which does it\n\
       --apply                       With --file-op, do it: move the files, then write the\n\
                                     document, putting the moves back if the write fails\n\n\
       --configuration <file>        Show which target would build this project, what each\n\
                                     value came from, and what this machine overrode\n\
       --target <name>               With --configuration, resolve for this target\n\
       --override <key>=<value>      With --configuration, set `profile` or `features` for this\n\
                                     run; a list is comma-separated\n\
       --remember                    With --configuration, record the target and the overrides\n\
                                     for this project on this machine\n\
       --forget                      With --configuration, forget everything recorded for it\n\n\
       --import <file-or-directory>  Read an existing project, say what it is, and what an\n\
                                     import would change; --apply migrates it, keeping the file\n\
                                     that was read, and --check-backup reports what was kept\n\
       --check-backup                With --import, report the files a past migration kept\n\
       --validate <file>             Check what the project needs and whether it is there, and\n\
                                     change nothing: the document is read, never written\n\
       --diagnose                    With --validate or --open-project, read the toolchain, which\n\
                                     runs two programs and needs the project to be trusted\n\
       --record <file>               With --validate, write a scan of the project to <file>, so a\n\
                                     later --rescan has something to compare against\n\
       --rescan <file>               With --validate, compare the project with the scan recorded in\n\
                                     <file> and name what appeared, vanished or changed\n\
       --kit <file>                  Show which tools a kit is made of, and what this machine has\n\
                                     of them; --diagnose asks it about each member (runs the\n\
                                     programs the kit names), --triple says which output to ask about\n\
       --triple <triple>             With --kit, the output triple to ask about: a kit never\n\
                                     carries which profile is being built\n\
       --new-workspace <directory>   Create a workspace document in a directory, with no members\n\
                                     yet; --name <name> (default: the directory)\n\
       --workspace <file>            Show a workspace: its members, what each holds, and its files\n\
                                     qualified by the member that names them\n\
       --add <name>=<root>           With --workspace, name a root that is already there as a\n\
                                     member; without --apply this only plans it\n\
       --remove <name>               With --workspace, take a member out of the document; its\n\
                                     directory and everything in it stay where they are\n\
       --activate <name>             With --workspace, record which member is active on this\n\
                                     machine, which the document does not say\n\
       --file <member>:<path>        With --workspace, show that one file: two members can each\n\
                                     hold a path of the same name\n\
     Global options:\n\
       --log-level <level>           error | warn | info | debug | trace (default: info)\n"
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Command, ConfigurationOptions, DropEntryOptions, FileOpOptions, ImportOptions, KitOptions,
        LaunchOptions, NewProjectOptions, NewWorkspaceOptions, OpenProjectOptions, ValidateOptions,
        WorkspaceAction, WorkspaceOptions, log_level_of, parse,
    };
    use crate::host::log::Level;
    use crate::project::{configuration::SettingValue, create::Template, files::FileOperation};
    use std::time::Duration;

    #[test]
    fn no_arguments_show_status() {
        assert_eq!(parse([] as [&str; 0]), Ok(Command::Status));
    }

    #[test]
    fn parses_help_and_version_options() {
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["-h"]), Ok(Command::Help));
        assert_eq!(parse(["--version"]), Ok(Command::Version));
        assert_eq!(parse(["-V"]), Ok(Command::Version));
    }

    #[test]
    fn parses_the_icon_export_directory() {
        assert_eq!(
            parse(["--render-icons", "out/icons"]),
            Ok(Command::RenderIcons(PathBuf::from("out/icons")))
        );
    }

    #[test]
    fn parses_the_probe_report_directory() {
        assert_eq!(
            parse(["--probe-apis", "out/probe"]),
            Ok(Command::ProbeApis(PathBuf::from("out/probe")))
        );
        assert_eq!(
            parse(["--probe-host", "out/host"]),
            Ok(Command::ProbeHost(PathBuf::from("out/host")))
        );
        assert_eq!(
            parse(["--project-check", "demo/project.json"]),
            Ok(Command::ProjectCheck(PathBuf::from("demo/project.json")))
        );
    }

    #[test]
    fn parses_the_new_project_options() {
        assert_eq!(
            parse(["--new-project", "demo"]),
            Ok(Command::NewProject(NewProjectOptions::new(
                PathBuf::from("demo"),
                Template::BasicPage
            )))
        );
        assert_eq!(
            parse([
                "--new-project",
                "demo",
                "--template",
                "application",
                "--name",
                "My App"
            ]),
            Ok(Command::NewProject(NewProjectOptions {
                directory: PathBuf::from("demo"),
                name: Some("My App".to_owned()),
                template: Template::Application,
            }))
        );

        // A template this build does not have, and a flag nobody knows, are usage errors.
        assert!(parse(["--new-project", "demo", "--template", "nowhere"]).is_err());
        assert!(parse(["--new-project"]).is_err());
        assert!(parse(["--new-project", "demo", "--template"]).is_err());
        assert!(parse(["--new-project", "demo", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_open_project_options() {
        assert_eq!(
            parse(["--open-project", "demo/project.json"]),
            Ok(Command::OpenProject(OpenProjectOptions::new(
                PathBuf::from("demo/project.json")
            )))
        );
        assert_eq!(
            parse([
                "--open-project",
                "demo/project.json",
                "--trust",
                "--diagnose"
            ]),
            Ok(Command::OpenProject(OpenProjectOptions {
                path: PathBuf::from("demo/project.json"),
                trust: true,
                diagnose: true,
            }))
        );
        assert_eq!(parse(["--recent"]), Ok(Command::Recent));
        assert!(parse(["--open-project"]).is_err());
        assert!(parse(["--open-project", "demo/project.json", "--wat"]).is_err());
        assert!(parse(["--recent", "extra"]).is_err());
    }

    #[test]
    fn parses_the_project_tree_and_the_entry_removal() {
        assert_eq!(
            parse(["--project-tree", "demo/project.json"]),
            Ok(Command::ProjectTree(PathBuf::from("demo/project.json")))
        );
        assert_eq!(
            parse([
                "--drop-entry",
                "demo/project.json",
                "--document",
                "pages/main.json"
            ]),
            Ok(Command::DropEntry(DropEntryOptions::new(
                PathBuf::from("demo/project.json"),
                "pages/main.json"
            )))
        );
        assert_eq!(
            parse([
                "--drop-entry",
                "demo/project.json",
                "--document",
                "pages/main.json",
                "--fix",
                "--trust"
            ]),
            Ok(Command::DropEntry(DropEntryOptions {
                path: PathBuf::from("demo/project.json"),
                document: "pages/main.json".to_owned(),
                fix: true,
                trust: true,
            }))
        );
        assert!(parse(["--project-tree"]).is_err());
        assert!(parse(["--drop-entry", "demo/project.json"]).is_err());
        assert!(parse(["--drop-entry", "demo/project.json", "--wat"]).is_err());
    }

    #[test]
    fn the_log_level_option_is_read_and_removed_from_the_command() {
        assert_eq!(log_level_of(["--help"]).unwrap(), None);
        assert_eq!(
            log_level_of(["--log-level", "debug", "--launch"]).unwrap(),
            Some(Level::Debug)
        );
        assert_eq!(
            parse(["--log-level", "warn", "--launch"]),
            Ok(Command::Launch(LaunchOptions::default()))
        );
        assert_eq!(
            parse(["--launch", "--log-level", "trace", "--exit-after-ms", "10"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(10)
            })),
        );

        // A missing or unknown level is a usage error, not a silent fallback to the default.
        assert!(log_level_of(["--log-level"]).is_err());
        assert!(log_level_of(["--log-level", "shout"]).is_err());
        assert!(parse(["--log-level", "shout", "--help"]).is_err());
    }

    #[test]
    fn parses_the_import_options() {
        assert_eq!(
            parse(["--import", "demo/project.json"]),
            Ok(Command::Import(ImportOptions::new(PathBuf::from(
                "demo/project.json"
            ))))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}import", "demo", "--apply", "--trust"]),
            Ok(Command::Import(ImportOptions {
                path: PathBuf::from("demo"),
                apply: true,
                check_backup: false,
                trust: true,
            }))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}import", "demo", "--check-backup"]),
            Ok(Command::Import(ImportOptions {
                path: PathBuf::from("demo"),
                apply: false,
                check_backup: true,
                trust: false,
            }))
        );

        // A missing path, an unknown flag, and asking to migrate and to only report at the same time
        // are usage errors: none of them can be read as "do nothing", which would look like success.
        assert!(parse(["\u{2d}\u{2d}import"]).is_err());
        assert!(parse(["\u{2d}\u{2d}import", "demo", "--wat"]).is_err());
        assert!(parse(["\u{2d}\u{2d}import", "demo", "--apply", "--check-backup"]).is_err());
    }

    #[test]
    fn parses_the_kit_options() {
        assert_eq!(
            parse(["--kit", "area/kit.json"]),
            Ok(Command::Kit(KitOptions::new(PathBuf::from(
                "area/kit.json"
            ))))
        );
        assert_eq!(
            parse([
                "--kit",
                "area/kit.json",
                "--diagnose",
                "--trust",
                "--triple",
                "x86_64-unknown-linux-gnu",
            ]),
            Ok(Command::Kit(KitOptions {
                path: PathBuf::from("area/kit.json"),
                diagnose: true,
                trust: true,
                triple: Some("x86_64-unknown-linux-gnu".to_owned()),
            }))
        );

        // A missing path, an unknown flag, an empty triple and a value that is not there are usage
        // errors: none of them can be read as "it is fine", which is what a silent default would be.
        assert!(parse(["--kit"]).is_err());
        assert!(parse(["--kit", "area/kit.json", "--wat"]).is_err());
        assert!(parse(["--kit", "area/kit.json", "--triple", "  "]).is_err());
        assert!(parse(["--kit", "area/kit.json", "--triple"]).is_err());
    }

    #[test]
    fn parses_the_workspace_options() {
        assert_eq!(
            parse(["--workspace", "area/workspace.json"]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::Report
            )))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "app=members/app",
                "--apply",
                "--trust"
            ]),
            Ok(Command::Workspace(WorkspaceOptions {
                path: PathBuf::from("area/workspace.json"),
                action: WorkspaceAction::Add {
                    name: "app".to_owned(),
                    root: "members/app".to_owned(),
                },
                apply: true,
                trust: true,
            }))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--remove",
                "lib"
            ]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::Remove {
                    name: "lib".to_owned(),
                }
            )))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--activate",
                "lib"
            ]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::Activate {
                    name: "lib".to_owned(),
                }
            )))
        );
        assert_eq!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--file",
                "lib:pages/main.json"
            ]),
            Ok(Command::Workspace(WorkspaceOptions::new(
                PathBuf::from("area/workspace.json"),
                WorkspaceAction::File {
                    name: "lib:pages/main.json".to_owned(),
                }
            )))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}new-workspace", "area", "--name", "Area"]),
            Ok(Command::NewWorkspace(NewWorkspaceOptions {
                directory: PathBuf::from("area"),
                name: Some("Area".to_owned()),
            }))
        );

        // A name that cannot be addressed, a pair with no `=`, a path with no member, and a missing
        // value are all usage errors: none of them can be read as "do nothing", which looks like
        // success.
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "app"
            ])
            .is_err()
        );
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "=members/app"
            ])
            .is_err()
        );
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--add",
                "two members=x"
            ])
            .is_err()
        );
        assert!(parse(["\u{2d}\u{2d}workspace", "area/workspace.json", "--remove"]).is_err());
        assert!(
            parse([
                "\u{2d}\u{2d}workspace",
                "area/workspace.json",
                "--file",
                "pages/main.json"
            ])
            .is_err()
        );
        assert!(parse(["\u{2d}\u{2d}workspace"]).is_err());
        assert!(parse(["\u{2d}\u{2d}new-workspace"]).is_err());
    }

    #[test]
    fn parses_the_validate_options() {
        assert_eq!(
            parse(["--validate", "demo/project.json"]),
            Ok(Command::Validate(ValidateOptions::new(PathBuf::from(
                "demo/project.json"
            ))))
        );
        assert_eq!(
            parse([
                "--validate",
                "demo/project.json",
                "--diagnose",
                "--trust",
                "--record",
                "out/scan.json",
            ]),
            Ok(Command::Validate(ValidateOptions {
                path: PathBuf::from("demo/project.json"),
                diagnose: true,
                trust: true,
                record: Some(PathBuf::from("out/scan.json")),
                rescan: None,
            }))
        );
        assert_eq!(
            parse(["--validate", "demo", "--rescan", "out/scan.json"]),
            Ok(Command::Validate(ValidateOptions {
                path: PathBuf::from("demo"),
                diagnose: false,
                trust: false,
                record: None,
                rescan: Some(PathBuf::from("out/scan.json")),
            }))
        );

        // A missing path, an unknown flag, and a flag whose value is not there are usage errors:
        // none of them can be read as "check it and change nothing", which would look like success.
        assert!(parse(["--validate"]).is_err());
        assert!(parse(["--validate", "demo", "--wat"]).is_err());
        assert!(parse(["--validate", "demo", "--record"]).is_err());
        assert!(parse(["--validate", "demo", "--rescan"]).is_err());
    }

    #[test]
    fn parses_the_configuration_options() {
        assert_eq!(
            parse(["--configuration", "demo/project.json"]),
            Ok(Command::Configuration(ConfigurationOptions::new(
                PathBuf::from("demo/project.json")
            )))
        );
        assert_eq!(
            parse([
                "--configuration",
                "demo/project.json",
                "--target",
                "release",
                "--override",
                "profile=release",
                "--override",
                "features=desktop,gpu",
                "--remember"
            ]),
            Ok(Command::Configuration(ConfigurationOptions {
                path: PathBuf::from("demo/project.json"),
                target: Some("release".to_owned()),
                overrides: vec![
                    super::OverrideWord {
                        key: "profile".to_owned(),
                        value: SettingValue::Word("release".to_owned()),
                    },
                    super::OverrideWord {
                        key: "features".to_owned(),
                        value: SettingValue::List(vec!["desktop".to_owned(), "gpu".to_owned()]),
                    },
                ],
                remember: true,
                forget: false,
            }))
        );
        assert_eq!(
            parse(["--configuration", "p.json", "--forget"]),
            Ok(Command::Configuration(ConfigurationOptions {
                path: PathBuf::from("p.json"),
                target: None,
                overrides: Vec::new(),
                remember: false,
                forget: true,
            }))
        );

        // A missing path, a missing value, an override without `=` and an unknown flag are usage
        // errors: none of them can be read as "record nothing", which would look like success.
        assert!(parse(["--configuration"]).is_err());
        assert!(parse(["--configuration", "p.json", "--target"]).is_err());
        assert!(parse(["--configuration", "p.json", "--override"]).is_err());
        assert!(parse(["--configuration", "p.json", "--override", "profile"]).is_err());
        assert!(parse(["--configuration", "p.json", "--override", "=release"]).is_err());
        assert!(parse(["--configuration", "p.json", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_file_operation_options() {
        assert_eq!(
            parse(["--file-op", "demo/project.json", "--add", "assets/logo.svg"]),
            Ok(Command::FileOp(FileOpOptions::new(
                PathBuf::from("demo/project.json"),
                FileOperation::Add {
                    path: "assets/logo.svg".to_owned()
                }
            )))
        );
        assert_eq!(
            parse(["\u{2d}\u{2d}file-op", "p.json", "--x"]),
            Err(super::ParseError {
                argument: std::ffi::OsString::from("--x")
            })
        );

        // `--move` takes two paths; `--rename` is the same operation under the name a person uses
        // when the file does not change places.
        for spelling in ["--move", "--rename"] {
            assert_eq!(
                parse(["--file-op", "p.json", spelling, "a.svg", "b.svg"]),
                Ok(Command::FileOp(FileOpOptions::new(
                    PathBuf::from("p.json"),
                    FileOperation::Move {
                        from: "a.svg".to_owned(),
                        to: "b.svg".to_owned()
                    }
                )))
            );
        }
        assert_eq!(
            parse([
                "--file-op",
                "p.json",
                "--delete",
                "a.svg",
                "--apply",
                "--trust"
            ]),
            Ok(Command::FileOp(FileOpOptions {
                path: PathBuf::from("p.json"),
                operation: FileOperation::Delete {
                    path: "a.svg".to_owned()
                },
                apply: true,
                trust: true,
            }))
        );

        // A missing path, a missing operation, a half-given move and an unknown flag are usage
        // errors: none of them can be read as "do nothing", which would look like success.
        assert!(parse(["--file-op"]).is_err());
        assert!(parse(["--file-op", "p.json"]).is_err());
        assert!(parse(["--file-op", "p.json", "--add"]).is_err());
        assert!(parse(["--file-op", "p.json", "--move", "a.svg"]).is_err());
        assert!(parse(["--file-op", "p.json", "--move", "a.svg", "b.svg", "--wat"]).is_err());
    }

    #[test]
    fn parses_the_baseline_session_and_its_duration() {
        // `--launch` used to be an unknown argument; it is a command now, and its default session
        // is bounded so a smoke test can end on its own.
        assert_eq!(
            parse(["--launch"]),
            Ok(Command::Launch(LaunchOptions::default()))
        );
        assert_eq!(
            parse(["--launch", "--exit-after-ms", "1500"]),
            Ok(Command::Launch(LaunchOptions {
                exit_after: Duration::from_millis(1500)
            })),
        );
        assert_ne!(
            LaunchOptions::default().exit_after,
            Duration::from_millis(1500)
        );

        // A duration that is missing, unparsable or followed by anything else is a usage error.
        assert!(parse(["--launch", "--exit-after-ms"]).is_err());
        assert!(parse(["--launch", "--exit-after-ms", "soon"]).is_err());
        assert!(parse(["--launch", "--exit-after-ms", "10", "extra"]).is_err());
        assert!(parse(["--launch", "--unknown"]).is_err());
    }

    #[test]
    fn rejects_unknown_and_extra_arguments() {
        assert!(parse(["--help", "--version"]).is_err());
        // A missing or repeated directory is a usage error rather than an empty path: exporting
        // into "" would write into the process's working directory.
        assert!(parse(["--render-icons"]).is_err());
        assert!(parse(["--render-icons", "a", "b"]).is_err());
        assert!(parse(["--probe-apis"]).is_err());
        assert!(parse(["--probe-apis", "a", "b"]).is_err());
        assert!(parse(["--probe-host"]).is_err());
        assert!(parse(["--probe-host", "a", "b"]).is_err());
    }
}
