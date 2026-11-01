// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The optional AI seam: context authorization, structured candidates and human review (task F23).
//!
//! [`workflow`] holds the role, context, candidate, review and workspace types; [`mcp`] is the stdio
//! transport an authorized tool call goes through; [`journal`] is the durable session that survives a
//! restart and refuses a stale writer. This boundary re-exports them.

mod journal;
mod mcp;
mod workflow;

pub use journal::{JOURNAL_VERSION, Journal, SessionRecord};
pub use mcp::{DEFAULT_REQUEST_TIMEOUT, McpServer};
pub use workflow::*;
