// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Guarding protected regions against every way text is written (task T06.06).
//!
//! The acceptance line is *"every write entry point has positive and negative cases"*. A guard that
//! answers once is easy to bypass by using a different door, so the module names the doors and answers
//! for all of them:
//!
//! * **The entry points are enumerated, not assumed.** [`WriteSource`] lists the five ways text changes
//!   — [`Input`](WriteSource::Input), [`Paste`](WriteSource::Paste), [`Replace`](WriteSource::Replace),
//!   [`Format`](WriteSource::Format) and [`External`](WriteSource::External) — so a new way of writing
//!   has to be added here and cannot quietly skip the guard.
//! * **A protected region is a range a generator owns.** A *replacement* that overlaps such a range is
//!   refused, and so is an *insertion* strictly inside it; typing **at** either boundary is allowed, so
//!   a person can still write on both sides of a generated block.
//! * **One refusal refuses the whole write.** [`WriteGuard::check`] walks the edits of a single entry
//!   point and returns the first offence, applying nothing — the same all-or-nothing shape as the
//!   store's [`crate::project::text::TextStore::edit_many`], so a paste cannot half-apply.
//! * **Read-only stops the editor, not the world.** The document's [`Writability`] refuses the
//!   editor's own writes; an external change is not the editor's to refuse, so it is checked against
//!   the protected regions and reported as a fact instead of being blocked.
//!
//! The seam to the permission model is deliberate: this is the same protection that
//! [`Capability::EditGeneratedCode`](crate::contracts::permission::Capability::EditGeneratedCode) is
//! `AgentReadOnly` for, so the guard names the region rather than inventing a second vocabulary.

#![allow(dead_code)]

use crate::project::conflict::Writability;

/// Which door a write came through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteSource {
    /// Typing, or any other per-keystroke insertion.
    Input,
    /// A paste, which may insert many bytes at once.
    Paste,
    /// A replace (a single replacement, or one step of a replace-all).
    Replace,
    /// A formatter rewriting a region it was told to format.
    Format,
    /// A change found on disk, which the editor did not make.
    External,
}

impl WriteSource {
    /// Whether this is a write the editor makes (and read-only therefore refuses).
    pub fn is_editor_write(&self) -> bool {
        !matches!(self, Self::External)
    }

    /// The source's name, as a command line writes it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Paste => "paste",
            Self::Replace => "replace",
            Self::Format => "format",
            Self::External => "external",
        }
    }

    /// The source a name picks.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "input" => Some(Self::Input),
            "paste" => Some(Self::Paste),
            "replace" => Some(Self::Replace),
            "format" => Some(Self::Format),
            "external" => Some(Self::External),
            _ => None,
        }
    }
}

/// A byte range a generator owns and a person must not write into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProtectedRegion {
    start: usize,
    end: usize,
}

impl ProtectedRegion {
    /// A region over `[start, end)`.
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start: start.min(end),
            end: start.max(end),
        }
    }

    /// Where it starts.
    pub fn start(&self) -> usize {
        self.start
    }

    /// Where it ends.
    pub fn end(&self) -> usize {
        self.end
    }

    /// How many bytes it covers.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether a range overlaps it.
    pub fn overlaps(&self, at: usize, remove: usize) -> bool {
        at < self.end && at + remove > self.start
    }

    /// The region as one line.
    pub fn render(&self) -> String {
        format!(
            "protected {}..{} ({} byte(s))",
            self.start,
            self.end,
            self.len()
        )
    }
}

/// One write: the range it replaces, and how many bytes it inserts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Write {
    /// Where the write starts.
    pub at: usize,
    /// How many bytes it removes.
    pub remove: usize,
    /// How many bytes it inserts.
    pub insert: usize,
}

impl Write {
    /// An insertion of `insert` bytes at a point.
    pub const fn insert(at: usize, insert: usize) -> Self {
        Self {
            at,
            remove: 0,
            insert,
        }
    }

    /// A replacement of `remove` bytes with `insert` bytes.
    pub const fn replace(at: usize, remove: usize, insert: usize) -> Self {
        Self { at, remove, insert }
    }

    /// A deletion of `remove` bytes.
    pub const fn delete(at: usize, remove: usize) -> Self {
        Self {
            at,
            remove,
            insert: 0,
        }
    }

    /// Whether this write changes a protected region.
    ///
    /// A replacement that overlaps the region changes it. An insertion changes it only when it lands
    /// strictly inside: inserting at the region's start or end writes beside the block, not into it.
    pub fn touches(&self, region: &ProtectedRegion) -> bool {
        if self.remove > 0 {
            region.overlaps(self.at, self.remove)
        } else if self.insert > 0 {
            self.at > region.start && self.at < region.end
        } else {
            false
        }
    }
}

/// Why the guard refused a write.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WriteRefusal {
    /// The document cannot be written by the editor at all.
    ReadOnly {
        /// The reason the document is read-only.
        reason: String,
    },
    /// The write would change a region a generator owns.
    Protected {
        /// Which door it came through.
        source: WriteSource,
        /// Where the write started.
        at: usize,
        /// The region it would change.
        region_start: usize,
        /// The end of the region it would change.
        region_end: usize,
    },
}

impl WriteRefusal {
    /// The refusal in one sentence.
    pub fn message(&self) -> String {
        match self {
            Self::ReadOnly { reason } => format!("the document is read-only: {reason}"),
            Self::Protected {
                source,
                at,
                region_start,
                region_end,
            } => format!(
                "the {} write at {at} would change the protected region {region_start}..{region_end}",
                source.name()
            ),
        }
    }
}

/// The regions a generator owns, and whether the editor may write at all.
#[derive(Clone, Debug)]
pub struct WriteGuard {
    regions: Vec<ProtectedRegion>,
    writability: Writability,
}

impl WriteGuard {
    /// A guard over some regions and a writability.
    pub fn new(regions: Vec<ProtectedRegion>, writability: Writability) -> Self {
        Self {
            regions,
            writability,
        }
    }

    /// A guard with no protected regions over a writable document.
    pub fn open() -> Self {
        Self::new(Vec::new(), Writability::Writable)
    }

    /// The protected regions.
    pub fn regions(&self) -> &[ProtectedRegion] {
        &self.regions
    }

    /// Whether the editor may write at all.
    pub fn writability(&self) -> &Writability {
        &self.writability
    }

    /// Whether a write through one door is allowed, and if not, why.
    pub fn check(&self, source: WriteSource, writes: &[Write]) -> Result<(), WriteRefusal> {
        if source.is_editor_write()
            && let Writability::ReadOnly { reason } = &self.writability
        {
            return Err(WriteRefusal::ReadOnly {
                reason: reason.clone(),
            });
        }
        for write in writes {
            for region in &self.regions {
                if write.touches(region) {
                    return Err(WriteRefusal::Protected {
                        source,
                        at: write.at,
                        region_start: region.start,
                        region_end: region.end,
                    });
                }
            }
        }
        Ok(())
    }

    /// The guard in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "guard: {} protected region(s), {}",
            self.regions.len(),
            self.writability.message()
        )];
        for region in &self.regions {
            lines.push(format!("  {}", region.render()));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{ProtectedRegion, Write, WriteGuard, WriteRefusal, WriteSource};
    use crate::project::conflict::Writability;

    fn guarded() -> WriteGuard {
        WriteGuard::new(vec![ProtectedRegion::new(4, 10)], Writability::Writable)
    }

    fn read_only() -> WriteGuard {
        WriteGuard::new(
            vec![ProtectedRegion::new(4, 10)],
            Writability::ReadOnly {
                reason: "declared read-only".to_owned(),
            },
        )
    }

    #[test]
    fn every_editor_entry_point_is_refused_inside_a_protected_region() {
        let guard = guarded();
        let inside = Write::insert(6, 1);
        for source in [
            WriteSource::Input,
            WriteSource::Paste,
            WriteSource::Replace,
            WriteSource::Format,
            WriteSource::External,
        ] {
            match guard.check(source, &[inside]) {
                Err(WriteRefusal::Protected { source: got, .. }) => assert_eq!(got, source),
                other => panic!("{source:?} was not refused: {other:?}"),
            }
        }
    }

    #[test]
    fn every_editor_entry_point_is_allowed_outside_a_protected_region() {
        let guard = guarded();
        let outside = Write::insert(20, 1);
        for source in [
            WriteSource::Input,
            WriteSource::Paste,
            WriteSource::Replace,
            WriteSource::Format,
            WriteSource::External,
        ] {
            assert!(
                guard.check(source, &[outside]).is_ok(),
                "{source:?} was refused outside"
            );
        }
    }

    #[test]
    fn typing_at_either_boundary_is_allowed_but_inside_is_not() {
        let guard = guarded();
        // At the region's start writes before the block; at its end writes after it.
        assert!(
            guard
                .check(WriteSource::Input, &[Write::insert(4, 1)])
                .is_ok()
        );
        assert!(
            guard
                .check(WriteSource::Input, &[Write::insert(10, 1)])
                .is_ok()
        );
        assert!(
            guard
                .check(WriteSource::Input, &[Write::insert(5, 1)])
                .is_err()
        );
        assert!(
            guard
                .check(WriteSource::Input, &[Write::insert(9, 1)])
                .is_err()
        );
    }

    #[test]
    fn a_replacement_that_overlaps_is_refused_even_partly() {
        let guard = guarded();
        // Deleting the whole region.
        assert!(
            guard
                .check(WriteSource::Replace, &[Write::delete(4, 6)])
                .is_err()
        );
        // A replacement that starts before the region and ends inside it.
        assert!(
            guard
                .check(WriteSource::Format, &[Write::replace(3, 3, 3)])
                .is_err()
        );
        // A replacement that starts inside and ends after it.
        assert!(
            guard
                .check(WriteSource::Format, &[Write::replace(8, 5, 5)])
                .is_err()
        );
        // A replacement entirely before it is fine.
        assert!(
            guard
                .check(WriteSource::Format, &[Write::replace(0, 4, 8)])
                .is_ok()
        );
    }

    #[test]
    fn one_offence_refuses_the_whole_write() {
        let guard = guarded();
        // A replace-all whose first step is fine and whose second step lands inside the region.
        let writes = [
            Write::replace(0, 2, 2),
            Write::insert(7, 1),
            Write::replace(20, 2, 2),
        ];
        assert!(guard.check(WriteSource::Replace, &writes).is_err());
    }

    #[test]
    fn read_only_refuses_the_editor_but_not_an_external_change() {
        let guard = read_only();
        let outside = Write::insert(20, 1);
        for source in [
            WriteSource::Input,
            WriteSource::Paste,
            WriteSource::Replace,
            WriteSource::Format,
        ] {
            match guard.check(source, &[outside]) {
                Err(WriteRefusal::ReadOnly { reason }) => assert_eq!(reason, "declared read-only"),
                other => panic!("{source:?} was not refused as read-only: {other:?}"),
            }
        }
        // An external change is not the editor's to refuse; it is only checked for the protected
        // region, and this one is outside it.
        assert!(guard.check(WriteSource::External, &[outside]).is_ok());
        // And an external change *inside* the region is still reported.
        assert!(
            guard
                .check(WriteSource::External, &[Write::delete(6, 1)])
                .is_err()
        );
    }

    #[test]
    fn an_empty_write_changes_nothing_and_is_allowed() {
        let guard = guarded();
        assert!(
            guard
                .check(WriteSource::Input, &[Write::insert(4, 0)])
                .is_ok()
        );
        assert!(
            guard
                .check(WriteSource::Input, &[Write::delete(6, 0)])
                .is_ok()
        );
    }
}
