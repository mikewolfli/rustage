// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The problem centre, design checks, test/visual evidence and the evidence bundle (task F21).
//!
//! [`report`] holds the types and operations; this boundary re-exports them. [`wizard`] is the test
//! wizard that turns declared state/navigation into data vectors run over both call seams (T21.05).

mod a11y;
mod junit;
mod report;
mod test_runner;
mod test_tree;
mod visual;
mod wizard;

pub use a11y::*;
pub use junit::*;
pub use report::*;
pub use test_runner::*;
pub use test_tree::*;
pub use visual::*;
pub use wizard::*;
