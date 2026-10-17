// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Migrating a state snapshot across a schema change, with an honest preview and an explicit reset
//! (task T17.05).
//!
//! The acceptance line is *"a missing migration blocks the automatic switch, and no field is lost while
//! pretending to be preserved"*:
//!
//! * **Every field is classified, none silently.** [`Change`] names a field *kept*, *added* with a
//!   default, *retyped*, or *dropped* with a reason. A field with no counterpart is [`Change::Dropped`],
//!   never shown as kept.
//! * **A loss blocks the automatic switch.** [`Migration::blocks_auto_switch`] is true only when a
//!   dropped field actually holds a value in the snapshot, so a schema drop that loses nothing does not
//!   block a switch, and one that would lose data does.
//! * **A downgrade is refused.** [`plan`] refuses a target version below the source's, because the
//!   migration table is written forwards only.
//! * **Resetting a generation that still owns tasks needs confirmation.** [`reset`] answers
//!   [`Refusal::NeedsConfirmation`] until the caller confirms, so a reset does not discard work nobody
//!   agreed to abandon.

#![allow(dead_code)]

use std::collections::BTreeMap;

use super::core::Value;

/// A state field's type.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldType {
    /// A yes/no field.
    Flag,
    /// A whole-number field.
    Number,
    /// A text field.
    Text,
}

impl FieldType {
    /// The type's name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Number => "number",
            Self::Text => "text",
        }
    }

    /// Reads a type's name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "flag" => Some(Self::Flag),
            "number" => Some(Self::Number),
            "text" => Some(Self::Text),
            _ => None,
        }
    }

    /// The value a field of this type starts at.
    pub fn default_value(&self) -> Value {
        match self {
            Self::Flag => Value::Flag(false),
            Self::Number => Value::Number(0),
            Self::Text => Value::Text(String::new()),
        }
    }

    /// Whether a value can be carried into this type at all.
    ///
    /// Only a step to text is a declared conversion: anything renders to text, and the migration names
    /// it as [`Change::Retyped`] so a lossy step is visible rather than silent.
    pub fn accepts_from(&self, from: Self) -> bool {
        self == &from || matches!(self, Self::Text)
    }
}

/// One field of a state schema.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Field {
    /// The field's name.
    pub name: String,
    /// Its type.
    pub kind: FieldType,
    /// The schema version it appeared in.
    pub since: u32,
}

/// A state schema: the fields a version of the logic holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Schema {
    /// The schema's version.
    pub version: u32,
    /// The fields.
    pub fields: Vec<Field>,
}

impl Schema {
    /// An empty schema at a version.
    pub fn new(version: u32) -> Self {
        Self {
            version,
            fields: Vec::new(),
        }
    }

    /// The same schema with one more field.
    pub fn with(mut self, name: impl Into<String>, kind: FieldType, since: u32) -> Self {
        self.fields.push(Field {
            name: name.into(),
            kind,
            since,
        });
        self
    }

    /// The field with a name, if the schema has one.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// A point-in-time state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Snapshot {
    /// The schema version it was taken under.
    pub version: u32,
    /// The values.
    pub values: BTreeMap<String, Value>,
}

impl Snapshot {
    /// An empty snapshot at a version.
    pub fn new(version: u32) -> Self {
        Self {
            version,
            values: BTreeMap::new(),
        }
    }

    /// The same snapshot with one more value.
    pub fn with(mut self, name: impl Into<String>, value: Value) -> Self {
        self.values.insert(name.into(), value);
        self
    }

    /// Whether the snapshot holds a value for a field.
    pub fn holds(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }
}

/// What happens to one field across a migration.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Change {
    /// The field exists in both schemas with the same type.
    Kept {
        /// The field.
        name: String,
    },
    /// The field is new in the target schema.
    Added {
        /// The field.
        name: String,
        /// What it starts at.
        default: Value,
    },
    /// The field exists in both but its type changed through a declared conversion.
    Retyped {
        /// The field.
        name: String,
        /// Its type before.
        from: FieldType,
        /// Its type after.
        to: FieldType,
    },
    /// The field has no target and is dropped.
    Dropped {
        /// The field.
        name: String,
        /// Why it is dropped.
        reason: String,
        /// Whether the snapshot actually held a value for it.
        held: bool,
    },
}

impl Change {
    /// The field the change is about.
    pub fn name(&self) -> &str {
        match self {
            Self::Kept { name }
            | Self::Added { name, .. }
            | Self::Retyped { name, .. }
            | Self::Dropped { name, .. } => name,
        }
    }

    /// The change in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Kept { name } => format!("{name}: kept"),
            Self::Added { name, default } => format!("{name}: added ({})", default.render()),
            Self::Retyped { name, from, to } => {
                format!("{name}: retyped {} -> {}", from.name(), to.name())
            }
            Self::Dropped { name, reason, held } => {
                let note = if *held { "value lost" } else { "no value held" };
                format!("{name}: dropped ({reason}; {note})")
            }
        }
    }
}

/// A migration from one schema version to another.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Migration {
    /// The source version.
    pub from: u32,
    /// The target version.
    pub to: u32,
    /// What happens to each field.
    pub changes: Vec<Change>,
}

impl Migration {
    /// The fields the migration keeps, adds or retypes.
    pub fn preserved(&self) -> Vec<&str> {
        self.changes
            .iter()
            .filter(|change| !matches!(change, Change::Dropped { .. }))
            .map(Change::name)
            .collect()
    }

    /// The fields the migration drops.
    pub fn dropped(&self) -> Vec<&str> {
        self.changes
            .iter()
            .filter_map(|change| match change {
                Change::Dropped { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The fields whose values the migration actually loses.
    pub fn lost(&self) -> Vec<&str> {
        self.changes
            .iter()
            .filter_map(|change| match change {
                Change::Dropped {
                    name, held: true, ..
                } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Whether a loss makes the automatic switch unsafe.
    pub fn blocks_auto_switch(&self) -> bool {
        !self.lost().is_empty()
    }

    /// The migration in one line per change.
    pub fn render(&self) -> Vec<String> {
        let mut lines = vec![format!("migration v{} -> v{}", self.from, self.to)];
        lines.extend(self.changes.iter().map(Change::render));
        lines
    }
}

/// Why a migration could not be planned.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Fault {
    /// The target version is older than the source's.
    Downgrade {
        /// The source version.
        from: u32,
        /// The target version.
        to: u32,
    },
}

impl Fault {
    /// The fault in one line.
    pub fn render(&self) -> String {
        match self {
            Self::Downgrade { from, to } => format!(
                "a migration from v{from} to v{to} goes backwards; the table is written forwards only"
            ),
        }
    }
}

/// Plans a migration from one schema to another against the snapshot it will carry.
pub fn plan(source: &Schema, target: &Schema, snapshot: &Snapshot) -> Result<Migration, Fault> {
    if target.version < source.version {
        return Err(Fault::Downgrade {
            from: source.version,
            to: target.version,
        });
    }
    let mut changes = Vec::new();
    for field in &source.fields {
        match target.field(&field.name) {
            Some(existing) if existing.kind == field.kind => changes.push(Change::Kept {
                name: field.name.clone(),
            }),
            Some(existing) if existing.kind.accepts_from(field.kind) => {
                changes.push(Change::Retyped {
                    name: field.name.clone(),
                    from: field.kind,
                    to: existing.kind,
                })
            }
            Some(existing) => changes.push(Change::Dropped {
                name: field.name.clone(),
                reason: format!(
                    "{} cannot be carried as {}",
                    field.kind.name(),
                    existing.kind.name()
                ),
                held: snapshot.holds(&field.name),
            }),
            None => changes.push(Change::Dropped {
                name: field.name.clone(),
                reason: format!("v{} has no field {}", target.version, field.name),
                held: snapshot.holds(&field.name),
            }),
        }
    }
    for field in &target.fields {
        if source.field(&field.name).is_none() {
            changes.push(Change::Added {
                name: field.name.clone(),
                default: field.kind.default_value(),
            });
        }
    }
    Ok(Migration {
        from: source.version,
        to: target.version,
        changes,
    })
}

/// Produces the migrated snapshot the preview described.
pub fn apply(migration: &Migration, snapshot: &Snapshot) -> Snapshot {
    let mut migrated = Snapshot::new(migration.to);
    for change in &migration.changes {
        match change {
            Change::Kept { name } => {
                if let Some(value) = snapshot.values.get(name) {
                    migrated.values.insert(name.clone(), value.clone());
                }
            }
            Change::Retyped { name, from, to } => {
                if let Some(value) = snapshot.values.get(name) {
                    let carried = if from == to {
                        value.clone()
                    } else {
                        Value::Text(value.render())
                    };
                    migrated.values.insert(name.clone(), carried);
                }
            }
            Change::Added { name, default } => {
                migrated.values.insert(name.clone(), default.clone());
            }
            Change::Dropped { .. } => {}
        }
    }
    migrated
}

/// The tasks a generation owns.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ownership {
    /// Who owns them.
    pub owner: String,
    tasks: Vec<String>,
}

impl Ownership {
    /// An owner with no tasks.
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            tasks: Vec::new(),
        }
    }

    /// The same ownership with one more task.
    pub fn own(mut self, task: impl Into<String>) -> Self {
        self.tasks.push(task.into());
        self
    }

    /// The tasks owned.
    pub fn tasks(&self) -> &[String] {
        &self.tasks
    }
}

/// What a reset discarded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reset {
    /// The tasks that went away with the generation.
    pub discarded: Vec<String>,
}

/// Why a reset was refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// The generation still owns tasks and nobody confirmed.
    NeedsConfirmation {
        /// The tasks that would be discarded.
        tasks: Vec<String>,
    },
}

impl Refusal {
    /// The refusal in one line.
    pub fn render(&self) -> String {
        match self {
            Self::NeedsConfirmation { tasks } => format!(
                "resetting would discard {} owned task(s): {}; confirm first",
                tasks.len(),
                tasks.join(", ")
            ),
        }
    }
}

/// Resets a generation, requiring confirmation while it still owns tasks.
pub fn reset(ownership: &Ownership, confirm: bool) -> Result<Reset, Refusal> {
    if !confirm && !ownership.tasks.is_empty() {
        return Err(Refusal::NeedsConfirmation {
            tasks: ownership.tasks.clone(),
        });
    }
    Ok(Reset {
        discarded: ownership.tasks.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Fault, FieldType, Ownership, Refusal, Schema, Snapshot, apply, plan, reset};
    use crate::app::logic::core::Value;

    fn v1() -> Schema {
        Schema::new(1)
            .with("count", FieldType::Number, 1)
            .with("label", FieldType::Text, 1)
    }

    #[test]
    fn a_kept_field_does_not_block_the_switch() {
        let snapshot = Snapshot::new(1).with("count", Value::Number(3));
        let migration = plan(&v1(), &v1(), &snapshot).expect("a plan");
        assert!(!migration.blocks_auto_switch());
        assert!(migration.dropped().is_empty());
        assert_eq!(migration.preserved(), vec!["count", "label"]);
    }

    #[test]
    fn a_dropped_field_that_holds_a_value_blocks_the_switch() {
        let snapshot = Snapshot::new(1)
            .with("count", Value::Number(3))
            .with("label", Value::Text("A".to_owned()));
        let target = Schema::new(2).with("count", FieldType::Number, 1);
        let migration = plan(&v1(), &target, &snapshot).expect("a plan");
        assert_eq!(migration.dropped(), vec!["label"]);
        assert_eq!(migration.lost(), vec!["label"]);
        assert!(migration.blocks_auto_switch());
        let rendered = migration.render().join("\n");
        assert!(rendered.contains("label: dropped"));
        assert!(rendered.contains("value lost"));
    }

    #[test]
    fn a_dropped_field_that_holds_nothing_is_reported_but_does_not_block() {
        let snapshot = Snapshot::new(1).with("count", Value::Number(3));
        let target = Schema::new(2).with("count", FieldType::Number, 1);
        let migration = plan(&v1(), &target, &snapshot).expect("a plan");
        assert_eq!(migration.dropped(), vec!["label"]);
        assert!(migration.lost().is_empty());
        assert!(!migration.blocks_auto_switch());
        assert!(migration.render().join("\n").contains("no value held"));
    }

    #[test]
    fn an_incompatible_retype_is_dropped_not_guessed() {
        let snapshot = Snapshot::new(1).with("field", Value::Text("7".to_owned()));
        let source = Schema::new(1).with("field", FieldType::Text, 1);
        let target = Schema::new(2).with("field", FieldType::Number, 1);
        let migration = plan(&source, &target, &snapshot).expect("a plan");
        assert_eq!(migration.dropped(), vec!["field"]);
        assert!(migration.blocks_auto_switch());
    }

    #[test]
    fn a_downgrade_is_refused() {
        let fault =
            plan(&Schema::new(3), &Schema::new(2), &Snapshot::new(3)).expect_err("a refusal");
        assert_eq!(fault, Fault::Downgrade { from: 3, to: 2 });
        assert!(fault.render().contains("forwards only"));
    }

    #[test]
    fn applying_a_migration_adds_defaults_and_drops_lost_fields() {
        let snapshot = Snapshot::new(1)
            .with("count", Value::Number(3))
            .with("label", Value::Text("A".to_owned()));
        let target =
            Schema::new(2)
                .with("count", FieldType::Number, 1)
                .with("ready", FieldType::Flag, 2);
        let migration = plan(&v1(), &target, &snapshot).expect("a plan");
        let migrated = apply(&migration, &snapshot);
        assert_eq!(migrated.version, 2);
        assert_eq!(migrated.values.get("count"), Some(&Value::Number(3)));
        assert_eq!(migrated.values.get("ready"), Some(&Value::Flag(false)));
        assert!(!migrated.holds("label"));
    }

    #[test]
    fn retyping_to_text_carries_the_rendered_value() {
        let snapshot = Snapshot::new(1).with("count", Value::Number(3));
        let source = Schema::new(1).with("count", FieldType::Number, 1);
        let target = Schema::new(2).with("count", FieldType::Text, 1);
        let migration = plan(&source, &target, &snapshot).expect("a plan");
        assert_eq!(migration.dropped(), Vec::<&str>::new());
        let migrated = apply(&migration, &snapshot);
        assert_eq!(
            migrated.values.get("count"),
            Some(&Value::Text("3".to_owned()))
        );
    }

    #[test]
    fn resetting_a_generation_that_owns_tasks_needs_confirmation() {
        let ownership = Ownership::new("gen-1").own("watcher").own("hot-build");
        let refusal = reset(&ownership, false).expect_err("a refusal");
        assert_eq!(
            refusal,
            Refusal::NeedsConfirmation {
                tasks: vec!["watcher".to_owned(), "hot-build".to_owned()],
            }
        );
        assert!(refusal.render().contains("confirm first"));

        let reset = reset(&ownership, true).expect("a reset");
        assert_eq!(reset.discarded.len(), 2);
    }

    #[test]
    fn resetting_an_owner_with_no_tasks_needs_no_confirmation() {
        let ownership = Ownership::new("gen-1");
        assert!(reset(&ownership, false).is_ok());
    }
}
