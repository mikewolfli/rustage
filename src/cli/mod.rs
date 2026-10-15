// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command line: what the executable accepts.
//!
//! This module is a boundary only; [`command`] holds the parser and the usage text.

mod command;

pub use command::{
    CloseOptions, Command, CommandOptions, ConfigurationOptions, ConflictOptions,
    DependenciesOptions, DocumentEdit, DocumentOptions, DropEntryOptions, EditOptions,
    FileOpOptions, FocusOptions, HistoryDomain, HistoryEdit, HistoryOptions, ImportOptions,
    KeysOptions, KitOptions, NewWorkspaceOptions, OpenProjectOptions, OperationOptions,
    PaletteOptions, RecoveryOptions, SaveOptions, TabsOptions, ValidateOptions, WorkbenchOptions,
    WorkspaceAction, WorkspaceOptions, help_text, log_level_of, parse,
};

/// The process exit codes, defined once in the host and re-exported so a command-line caller and a
/// script agree on the numbers without a second table.
pub use crate::host::error::ExitCode;
