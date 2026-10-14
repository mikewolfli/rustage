// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command line: what the executable accepts.
//!
//! This module is a boundary only; [`command`] holds the parser and the usage text.

mod command;

pub use command::{
    Command, ConfigurationOptions, DropEntryOptions, FileOpOptions, ImportOptions,
    OpenProjectOptions, ValidateOptions, help_text, log_level_of, parse,
};

/// The process exit codes, defined once in the host and re-exported so a command-line caller and a
/// script agree on the numbers without a second table.
pub use crate::host::error::ExitCode;
