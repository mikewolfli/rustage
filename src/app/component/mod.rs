// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Components, instances, variants, resources and template assets (feature family F12, roadmap
//! MS08).
//!
//! A component is not a synonym for copy and paste: the definition, an instance's overrides and the
//! generation that consumes them have to stay in a traceable relationship. Resource import carries
//! type, size, reference, path-safety and target limits, and promises no runtime support for an
//! arbitrary format. The family is split the way the work divides:
//!
//! * definitions, instances, parameters, slots and the cycle check ([`definition`]),
//! * extraction, insertion, definition edits and the affected instances ([`authoring`]),
//! * variants, local overrides, detach and reset, and target compatibility ([`variant`]),
//! * custom-widget registration and its publishability ([`custom`]),
//! * resource import, the reference index and transactional replace and move ([`resource`]),
//! * per-target resource variants and the capacity check ([`targets`]),
//! * package import plans, their refusals and cancellation ([`package`]),
//! * definition upgrade, cross-project paste and the static artifact ([`upgrade`]),
//! * the asset manifest, packaging and the derived-resource pipeline ([`manifest`]).
//!
//! Across all of them one rule holds: a failure is never shaped like a success. A missing target
//! implementation is [`custom::Publishable::No`] rather than a green check, a failed move keeps the
//! original path and its references, and a plan cancelled before commit has applied nothing.
pub mod authoring;
pub mod custom;
pub mod definition;
pub mod manifest;
pub mod package;
pub mod resource;
pub mod targets;
pub mod upgrade;
pub mod variant;
