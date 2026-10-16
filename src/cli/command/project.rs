// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The project commands: creating, opening, importing, validating and addressing a
//! project, its workspace, its kit and its dependencies.

// Every branch of the flag table answers with `return`, including the ones that could be a
// trailing expression: the branches are independent flags, and writing them alike is worth more
// than saving a keyword. The returns that are *not* needless — the ones inside a loop that refuses
// a bad value — are why the table reads the way it does.
#![allow(clippy::needless_return)]

use super::*;

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

/// How a workspace's dependencies should be resolved (task T02.11).
///
/// Showing the resolution runs nothing: it prints the command and the effects that command would
/// have. Nothing is started until every effect has been authorized, because a dependency command is
/// the one command in this build that can write a file or reach the network. The flags *are* those
/// effects: `--allow-lockfile-change` lets the resolution write the lockfile, `--allow-network` lets
/// it reach the registry, and neither asks for an upgrade — the resolution stays locked unless the
/// lockfile itself may change, and then the change is what the report is about.
#[derive(Debug, PartialEq, Eq)]
pub struct DependenciesOptions {
    /// The workspace directory: the one the manifest is in.
    pub directory: PathBuf,
    /// The record to compare the resolution against, when there is one.
    pub review: Option<PathBuf>,
    /// Where to write the record, when the caller asked for one.
    pub record: Option<PathBuf>,
    /// Whether the lockfile may be written.
    pub allow_lockfile_change: bool,
    /// Whether the registry may be reached.
    pub allow_network: bool,
    /// Whether the person answering the directory's question said yes.
    pub trust: bool,
}

impl DependenciesOptions {
    /// Options that resolve a workspace and change nothing.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            review: None,
            record: None,
            allow_lockfile_change: false,
            allow_network: false,
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

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_project(
    text: &str,
    argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    match text {
        "--new-project" => {
            let Some(directory) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = NewProjectOptions::new(PathBuf::from(directory), Template::BasicPage);
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--template" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(template) = value.to_str().and_then(Template::from_name) else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.template = template;
                    }
                    "--name" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.name = Some(name.to_owned());
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::NewProject(options)));
        }
        "--open-project" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = OpenProjectOptions::new(PathBuf::from(path));
            for argument in arguments.by_ref() {
                match argument.to_str().unwrap_or_default() {
                    "--trust" => options.trust = true,
                    "--diagnose" => options.diagnose = true,
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::OpenProject(options)));
        }
        "--recent" => {
            let rest = arguments;
            if let Some(argument) = rest.next() {
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::Recent));
        }
        "--project-tree" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let rest = arguments;
            if let Some(argument) = rest.next() {
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::ProjectTree(PathBuf::from(path))));
        }
        "--drop-entry" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = DropEntryOptions::new(PathBuf::from(path), "");
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--document" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(document) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.document = document.to_owned();
                    }
                    "--fix" => options.fix = true,
                    "--trust" => options.trust = true,
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            if options.document.is_empty() {
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::DropEntry(options)));
        }
        "--file-op" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let rest = arguments;
            let mut operation = None;
            let mut apply = false;
            let mut trust = false;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--add" | "--exclude" | "--delete" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
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
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(to) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let (Some(from), Some(to)) = (from.to_str(), to.to_str()) else {
                            return Some(Err(ParseError { argument }));
                        };
                        operation = Some(FileOperation::Move {
                            from: from.to_owned(),
                            to: to.to_owned(),
                        });
                    }
                    "--apply" => apply = true,
                    "--trust" => trust = true,
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            let Some(operation) = operation else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = FileOpOptions::new(PathBuf::from(path), operation);
            options.apply = apply;
            options.trust = trust;
            return Some(Ok(Command::FileOp(options)));
        }
        "--kit" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = KitOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--diagnose" => options.diagnose = true,
                    "--trust" => options.trust = true,
                    "--triple" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(triple) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if triple.trim().is_empty() {
                            return Some(Err(ParseError { argument: value }));
                        }
                        options.triple = Some(triple.trim().to_owned());
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Kit(options)));
        }
        "--dependencies" => {
            let Some(directory) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = DependenciesOptions::new(PathBuf::from(directory));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--allow-lockfile-change" => options.allow_lockfile_change = true,
                    "--allow-network" => options.allow_network = true,
                    "--trust" => options.trust = true,
                    "--review" | "--record" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if argument.to_str().unwrap_or_default() == "--review" {
                            options.review = Some(PathBuf::from(path));
                        } else {
                            options.record = Some(PathBuf::from(path));
                        }
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Dependencies(options)));
        }
        "--new-workspace" => {
            let Some(directory) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = NewWorkspaceOptions::new(PathBuf::from(directory));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--name" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.name = Some(name.to_owned());
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::NewWorkspace(options)));
        }
        "--workspace" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = WorkspaceOptions::new(PathBuf::from(path), WorkspaceAction::Report);
            let rest = arguments;
            while let Some(argument) = rest.next() {
                let flag = argument.to_str().unwrap_or_default();
                match flag {
                    "--add" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(pair) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        let Some((name, root)) = pair.split_once('=') else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if !name_is_addressable(name) || root.trim().is_empty() {
                            return Some(Err(ParseError { argument: value }));
                        }
                        options.action = WorkspaceAction::Add {
                            name: name.to_owned(),
                            root: root.to_owned(),
                        };
                    }
                    "--remove" | "--activate" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if !name_is_addressable(name) {
                            return Some(Err(ParseError { argument: value }));
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
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(name) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if WorkspaceFile::parse(name).is_err() {
                            return Some(Err(ParseError { argument: value }));
                        }
                        options.action = WorkspaceAction::File {
                            name: name.to_owned(),
                        };
                    }
                    "--apply" => options.apply = true,
                    "--trust" => options.trust = true,
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Workspace(options)));
        }
        "--validate" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = ValidateOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--diagnose" => options.diagnose = true,
                    "--trust" => options.trust = true,
                    "--record" | "--rescan" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(path) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if argument.to_str().unwrap_or_default() == "--record" {
                            options.record = Some(PathBuf::from(path));
                        } else {
                            options.rescan = Some(PathBuf::from(path));
                        }
                    }
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Validate(options)));
        }
        "--configuration" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = ConfigurationOptions::new(PathBuf::from(path));
            let rest = arguments;
            while let Some(argument) = rest.next() {
                match argument.to_str().unwrap_or_default() {
                    "--target" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(target) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        options.target = Some(target.trim().to_owned());
                    }
                    "--override" => {
                        let Some(value) = rest.next() else {
                            return Some(Err(ParseError { argument }));
                        };
                        let Some(word) = value.to_str() else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        let Some((key, value)) = word.split_once('=') else {
                            return Some(Err(ParseError { argument: value }));
                        };
                        if key.trim().is_empty() {
                            return Some(Err(ParseError {
                                argument: word.into(),
                            }));
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
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            return Some(Ok(Command::Configuration(options)));
        }
        "--import" => {
            let Some(path) = arguments.next() else {
                return Some(Err(ParseError { argument }));
            };
            let mut options = ImportOptions::new(PathBuf::from(path));
            for argument in arguments.by_ref() {
                match argument.to_str().unwrap_or_default() {
                    "--apply" => options.apply = true,
                    "--check-backup" => options.check_backup = true,
                    "--trust" => options.trust = true,
                    _ => return Some(Err(ParseError { argument })),
                }
            }
            if options.apply && options.check_backup {
                return Some(Err(ParseError { argument }));
            }
            return Some(Ok(Command::Import(options)));
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

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
    fn parses_the_dependency_options() {
        assert_eq!(
            parse(["--dependencies", "area"]),
            Ok(Command::Dependencies(DependenciesOptions::new(
                PathBuf::from("area")
            )))
        );
        assert_eq!(
            parse([
                "--dependencies",
                "area",
                "--review",
                "area/deps.json",
                "--record",
                "area/now.json",
                "--allow-lockfile-change",
                "--allow-network",
                "--trust",
            ]),
            Ok(Command::Dependencies(DependenciesOptions {
                directory: PathBuf::from("area"),
                review: Some(PathBuf::from("area/deps.json")),
                record: Some(PathBuf::from("area/now.json")),
                allow_lockfile_change: true,
                allow_network: true,
                trust: true,
            }))
        );
        assert!(parse(["--dependencies"]).is_err());
        assert!(parse(["--dependencies", "area", "--wat"]).is_err());
        assert!(parse(["--dependencies", "area", "--record"]).is_err());
        assert!(parse(["--dependencies", "area", "--review"]).is_err());
        // Neither flag is spelled in a way that could be mistaken for the other:
        assert!(parse(["--dependencies", "area", "--lockfile"]).is_err());
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
}
