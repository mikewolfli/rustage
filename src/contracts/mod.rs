// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The shared contracts of the design document, from roadmap task T00.05.
//!
//! Everything in the designer that has more than one consumer — the canvas, the object tree, the
//! property panel, the generator, the build service — has to agree on a handful of shapes:
//! what an object is, how a change is expressed, who may make it, which version a result came
//! from, where a problem points, and how a control maps to generated source. T00.05 fixes those
//! shapes before the features that use them exist, because the alternative is each feature
//! inventing its own and the designer growing several partial copies of the same document.
//!
//! | Contract | Concern | Module |
//! |---|---|---|
//! | `C-IDENTITY` | stable node and page ids, never reused | [`identity`] |
//! | `C-VERSION` | revisions, stamps and the vocabulary of staleness | [`revision`] |
//! | `C-MODEL` | pages, nodes, property values | [`model`] |
//! | `C-CHANGE` | change sets and their rejections | [`change`] |
//! | `C-PERMISSION` | capabilities, grants, denials, revocations | [`permission`] |
//! | `C-DIAGNOSTIC` | problems, their location, their staleness | [`diagnostics`] |
//! | `C-SOURCEMAP` | node ↔ generated source ranges | [`source_map`] |
//! | `C-STORE` | the one writable model, and read-only views of it | [`store`] |
//!
//! The acceptance line for T00.05 is "no second writable model across views and services". That is
//! a property of [`store`], and the module docs there say exactly how it holds and where it does
//! not (the id sequence is deliberately outside the revision).
//!
//! **What these modules are not.** They are not the document format (T02.01), not the project
//! store with text buffers and a valid-model version (T03.01), not the property schema (T12.04)
//! and not the generator (T15.x). Where a contract has to leave room for those, it says so in the
//! item's own documentation instead of guessing now and being corrected later.
//!
//! The permission table is the exception to "code first": it is a copy of a table that already
//! exists in whitepaper §12.5, and `node tools/contracts/check-contracts.mjs` compares both
//! directions so the copy cannot drift. The same check ties every invariant listed in
//! [`docs/plan/contracts.md`](../../../docs/plan/contracts.md) to the test that enforces it.
//!
//! There is deliberately no re-export list here. Every module that gains a consumer imports what
//! it needs by path; a re-export nobody uses is an unused-import warning bought with nothing.

// T00.05 fixes the shared shapes before the features that use them exist, so most of this tree is
// consumed by tests and by `--status` rather than by a running feature. The document format arrives
// in T02.01, the project store with text buffers in T03.01, the property schema in T12.04 and
// generated source mapping in T15.03; each of them will consume a part of this tree and delete
// nothing. Until then the "never used" lint is silenced once, here, with that reason — not by
// deleting contracts whose invariants are already enforced by tests (see `docs/plan/contracts.md`).
#![allow(dead_code)]

pub mod change;
pub mod diagnostics;
pub mod identity;
pub mod model;
pub mod permission;
pub mod revision;
pub mod source_map;
pub mod store;

/// The vocabulary this build implements, printed by `--status`. See [`vocabulary`].
pub mod vocabulary;
