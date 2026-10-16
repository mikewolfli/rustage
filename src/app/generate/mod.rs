// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Mode 2: turning the design document into Rust source (feature family F15, roadmap MS07).
//!
//! The generator is not one module but the seam between five things that each have their own
//! failure: the **plan** of what is written where ([`artifact`]), the **pipeline** that asks the
//! library's own generator for source ([`pipeline`]), the **mapping** from a node to the bytes it
//! became ([`mapping`]), the **merge** that keeps hand-written code across a regeneration
//! ([`merge`]), the **reverse** path that reads a description change back into a model candidate
//! ([`reverse`]), the **position** recalculator that keeps a mapping honest after formatting
//! ([`position`]), the **export** that commits or stages the result ([`export`]), and the
//! **golden** comparisons that pin the whole thing down ([`golden`]).
//!
//! Two properties hold across all of them, because they are the reasons the feature exists:
//!
//! * **Generation is a pure function of the document.** Nothing here reads a clock, an environment
//!   or a directory that the request did not name; the same request produces the same bytes
//!   (principle 16). The library's [`rust_widgets::designer::generate`] is the emitter, and this
//!   family never writes a second one (principle 59).
//! * **Nothing is silently dropped.** A name the target cannot build, a container over capacity, a
//!   style reference that will not resolve: each is reported and blocks generation rather than
//!   producing a file that looks complete (principle 13/22).
pub mod artifact;
pub mod document;
pub mod export;
pub mod golden;
pub mod mapping;
pub mod merge;
pub mod pipeline;
pub mod position;
pub mod reverse;
