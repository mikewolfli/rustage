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
//!   for, and the English pack is *derived* from the vocabulary so it cannot drift from what the
//!   interface shows.
//! * **Switching the interface language changes nothing else.** Nothing here reads or writes the design
//!   document, a resource identity or a generated artifact; the catalog is text, and the generated
//!   application's own language resources are a separate product boundary (T24.04).
pub mod catalog;
pub mod vocabulary;
