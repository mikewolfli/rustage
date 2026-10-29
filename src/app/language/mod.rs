// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT
//! The language service: one session per project, the protocol, completion, navigation,
//! diagnostics and the editors that read them.

pub mod analysis_config;
pub mod diagnostics;
pub mod features;
pub mod hints;
pub mod navigation;
pub mod protocol;
pub mod schema_docs;
pub mod service_health;
pub mod session;
pub mod target;
pub mod workspace_edit;
