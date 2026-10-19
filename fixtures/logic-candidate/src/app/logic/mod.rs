// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The shared core, channel and worker, included from the designer's sources.
//!
//! The five `..` steps reach the repository root from this directory; if the fixture is ever moved,
//! the build fails loudly rather than compiling an older copy.

#[path = "../../../../../src/app/logic/core.rs"]
pub mod core;

#[path = "../../../../../src/app/logic/ipc.rs"]
pub mod ipc;

#[path = "../../../../../src/app/logic/migration.rs"]
pub mod migration;

#[path = "../../../../../src/app/logic/worker.rs"]
pub mod worker;
