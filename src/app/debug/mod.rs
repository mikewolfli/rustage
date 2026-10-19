// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The debug client (feature family F19, roadmap MS11).
//!
//! The family does not implement a Rust debugger. It speaks the Debug Adapter Protocol to an
//! adapter the platform already has, and the acceptance line — *"basic breakpoints, variables,
//! threads and source mapping are real, and the hot-update boundary is clear"* — is carried by the
//! split:
//!
//! * [`adapter`] answers **what this machine actually has**. A missing or unlicensed adapter is a
//!   blocker with a remedy, never a silent fallback to a fake session.
//! * [`protocol`] is the wire: `Content-Length` framed JSON, one reader thread, bounded waits. A
//!   timeout is a timeout, not an empty answer.
//! * [`session`] is the state machine: launch versus attach, which command is legal in which phase,
//!   and the one rule that matters for safety — **detaching from a process this session did not
//!   start never terminates it**.
//! * [`breakpoints`] keeps breakpoints bound to a node, not to a line, so regenerating the source
//!   moves them with their node and an unmapped node stays unbound instead of pointing at a
//!   neighbouring line.
//! * [`inspect`] is the lazy side: threads, frames, scopes, paged variables, watches and evaluation
//!   under an explicit side-effect policy.
//! * [`mapping`] is the two-way navigation between the generated source and the document model.
//!   When a position is not inside a mapped span it says so; it does not guess.
//! * [`hotswap`] states the boundary with the logic swap: replacing logic ends the debug session,
//!   and nothing pretends the call stack or breakpoint bindings survived.
//! * [`recovery`] is what happens when the adapter exits, the map goes stale or the target is not
//!   supported: the session ends, the process this session owns is taken down, and the project is
//!   editable again.
//!
//! The advanced native surface (data/function breakpoints, memory, registers, disassembly, core
//! dumps, composite sessions) is T19.09/T19.10 and is not part of this build.
pub mod adapter;
pub mod breakpoints;
pub mod hotswap;
pub mod inspect;
pub mod mapping;
pub mod protocol;
pub mod recovery;
pub mod session;
