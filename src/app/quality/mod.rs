// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The problem centre, design checks, test/visual evidence and the evidence bundle (task F21).
//!
//! [`report`] holds the types and operations; this boundary re-exports them.

mod report;
mod test_runner;

pub use report::*;
pub use test_runner::*;
