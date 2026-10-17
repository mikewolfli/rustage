// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Collection controls, content editing, navigation containers and specialised views (feature family
//! F27, roadmap MS08).
//!
//! F27 edits **what is inside a control** — a table's rows and columns, a tree's items, a container's
//! pages, a cell's editor, a rich-text run — rather than where the control sits on the canvas, which
//! is a different family. The design schema and the *sample content* are saved in JSON; the real
//! runtime data comes from an adapter, so sample content never masquerades as business data.
//!
//! The family is split the way the content divides:
//!
//! * the items, columns and cell roles, and the sample/source line ([`content`]),
//! * the typed columns and the honest mapping of a source row ([`table`]),
//! * tree and list items, their hierarchy and their lazy loads ([`tree`]),
//! * cell delegates, commit, cancel and error states ([`delegate`]),
//! * the internal pages of tab, stack, scroll and split containers ([`pages`]),
//! * rich text, links, images and target limits ([`rich`]),
//! * the specialised views that are actually supported ([`view`]),
//! * and the end-to-end walk over the whole content path ([`regression`]).
//!
//! One rule holds across all of them: a capability the library lacks is a distinct outcome from a
//! value that failed validation, and neither looks like success. A sample is labelled as one; a row
//! that does not map is refused with its diagnoses rather than truncated; a denied link is refused by
//! the permission gate; an unsupported view is a stated reason, never a picture standing in for a
//! running feature.
pub mod content;
pub mod delegate;
pub mod pages;
pub mod regression;
pub mod rich;
pub mod table;
pub mod tree;
pub mod view;
