// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT
//! Text in place: the piece table, a file's bytes as editable text, and the protected regions
//! every write goes through.

pub mod piece_table;
pub mod protection;
pub mod text_file;
