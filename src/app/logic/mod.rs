// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Rust event logic: incremental build and controlled replacement of the business process (feature
//! family F17, roadmap MS09).
//!
//! F17 covers the half of "the design runs" that F16 does not: F16 builds and runs the delivered
//! program as a whole, and this family replaces *part* of a running application — its business
//! logic — without losing the state the person has built up. The roadmap splits the work along the
//! steps a replacement actually takes, and each step is one module:
//!
//! * the shared business core and its static call seam ([`core`]),
//! * the local typed IPC between the stable UI process and the replaceable one ([`ipc`]),
//! * the replaceable process itself, which serves that channel over any input/output ([`worker`]),
//! * action validation and the synchronous/high-frequency events that stay in the UI ([`actions`]),
//! * the debounced incremental build and the candidate artifact it produces ([`candidate`]),
//! * the state schema, its snapshot and the migration preview ([`migration`]),
//! * the ordered walk that replaces one version with another ([`swap`]),
//! * which interactions are ready and which late result belongs to which version ([`combination`]),
//! * and the event vectors and fault decisions that close the loop ([`regression`]).
//!
//! Across all of them one rule holds: a result is a value a caller can inspect, and a failure is never
//! shaped like a success. A build that is only a `check` is not a candidate; a paused swap that misses
//! its deadline aborts with the state it reached; a state field with no migration path is named as
//! dropped rather than shown as kept; and a frame from the generation being replaced is refused
//! instead of applied.
pub mod actions;
pub mod candidate;
pub mod combination;
pub mod core;
pub mod ipc;
pub mod migration;
pub mod regression;
pub mod swap;
pub mod worker;
