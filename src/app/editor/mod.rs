// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT
//! The text editor: the adapter over the library's `CodeEditor`, the host's highlight cache, the
//! snippets and structural helpers, and the regression harness. The widget owns text, carets,
//! history, folding and highlighting; the host modules here own only host policy.

pub mod adapter;
pub mod editing;
pub mod highlight;
pub mod regression;
pub mod scale;
pub mod snippets;
pub mod structural;
