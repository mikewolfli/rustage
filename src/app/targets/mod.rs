// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Multi-target configuration, the capability matrix and local artifact delivery (feature family
//! F18, roadmap MS10).
//!
//! The family is split the way the decisions divide:
//!
//! * one profile's configuration ([`configuration`]),
//! * the versioned capability snapshot and the markers a switch leaves ([`snapshot`]),
//! * the desktop build/behaviour matrix ([`matrix`]),
//! * tablet/mobile size, input, resources and evidence ([`devices`]),
//! * mini/embedded static and `no_std` applicability plus the link audit ([`restricted`]),
//! * local devices, package export, install plans and recovery ([`local`]),
//! * the build/deployment queue, artifacts and signing references ([`queue`]),
//! * and the five-profile certification report ([`certification`]).
//!
//! Two rules hold across the family, because they are what the milestone is about:
//!
//! * **A profile is not an operating system, and a preview is not a device.** The profile says what
//!   the output can do; the triple says which system runs it; evidence from a simulator or a host
//!   preview is recorded as exactly that and never upgraded to device certification (principle 2/8).
//! * **Unverified is a result.** A missing build, a check that is not a build, a simulator's run and
//!   a cell with no command all stay visible; none of them is rendered as a pass (principle 22/25).
pub mod certification;
pub mod configuration;
pub mod devices;
pub mod local;
pub mod matrix;
pub mod queue;
pub mod restricted;
pub mod snapshot;
