// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The optional AI seam: context authorization, structured candidates and human review (task F23).
//!
//! [`workflow`] holds the role, context, candidate, review and workspace types; this boundary
//! re-exports them.

mod workflow;

pub use workflow::*;
