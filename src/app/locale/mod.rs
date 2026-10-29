// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The designer's own interface language: the string vocabulary, the language pack and the catalog
//! that reads it (feature family F24, roadmap task T24.03).
//!
//! Principle #67 asks for three things, and this domain is where the designer's half of them lives:
//!
//! * **The interface is multilingual and follows the system by default.** The platform half — which
//!   languages exist, which one the system asks for, which are bundled — is
//!   [`crate::host::locale`]. Here, [`catalog::select`] turns a request or the system's language into a
//!   decision, and a language the build does not bundle is refused rather than approximated.
//! * **Only the languages the build ships are offered.** [`vocabulary`] is the identified string set;
//!   [`catalog::Catalog::for_language`] answers [`catalog::NotBundled`] for a language there is no pack
//!   for, and the packs are keyed by the same identified vocabulary: English is *derived* from it
//!   and Simplified Chinese ([`zh_hans`]) is translated against it, so neither can drift from what
//!   the interface shows.
//! * **Switching the interface language changes nothing else.** Nothing here reads or writes the design
//!   document, a resource identity or a generated artifact; the catalog is text, and the generated
//!   application's own language resources are a separate product boundary (T24.04).
//!
//! [`mod@format`] is the half of a language that is not a string: dates, digit grouping and list
//! order, with the collation rule named so a list never claims an ordering it does not have.
pub mod catalog;
pub mod format;
pub mod vocabulary;
pub mod zh_hans;
