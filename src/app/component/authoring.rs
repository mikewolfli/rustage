// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Extraction, insertion and definition edits, and the rule that an edit never clobbers an
//! instance's override (task T12.02).
//!
//! The acceptance line is *"editing a definition does not overwrite a legal instance override"*.
//! The design keeps that true with one resolution order and one honest report:
//!
//! * **[`Library::resolve`] merges definition values first and the instance's overrides second.** An
//!   override is the instance's explicit words, so it wins; a definition edit that lands under an
//!   override changes nothing visible at that instance, which is exactly the property the
//!   acceptance line names.
//! * **[`Library::edit`] reports what it touched.** It returns the affected instance identities and
//!   the overrides it preserved, so a caller never has to guess whether an edit silently rewrote a
//!   placed instance — it is told which ones it left alone, and why.
//! * **Extraction and insertion validate at the edge.** [`Library::extract`] refuses a blank or
//!   duplicate definition name, and [`Library::insert`] refuses an unknown definition and a
//!   duplicate instance identity, so a bad placement is unconstructable rather than merely
//!   discouraged.

#![allow(dead_code)]

use std::collections::BTreeMap;

use crate::contracts::identity::NodeId;
use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};

/// A definition built from a selection and edited over time.
#[derive(Clone, PartialEq, Debug)]
pub struct Definition {
    /// The definition's name.
    pub name: String,
    /// The version, bumped by every edit.
    pub version: u32,
    /// The root control kind the selection had.
    pub kind: NodeKind,
    /// The definition's values, keyed by property name.
    pub properties: BTreeMap<PropertyName, PropertyValue>,
}

/// A placed instance of a definition, with its explicit overrides.
#[derive(Clone, PartialEq, Debug)]
pub struct Instance {
    /// The definition this instance is of.
    pub definition: String,
    /// The instance's stable identity.
    pub id: NodeId,
    /// The values the instance overrides on top of the definition.
    pub overrides: BTreeMap<PropertyName, PropertyValue>,
}

/// One override a definition edit preserved.
#[derive(Clone, PartialEq, Debug)]
pub struct Preserved {
    /// The instance that holds it.
    pub instance: NodeId,
    /// The property the instance overrides.
    pub name: PropertyName,
    /// The value the instance keeps.
    pub value: PropertyValue,
}

/// What an edit changed and what it left alone.
#[derive(Clone, PartialEq, Debug)]
pub struct Edit {
    /// The instances of the edited definition, in identity order.
    pub affected: Vec<NodeId>,
    /// The overrides on those instances, preserved untouched.
    pub preserved: Vec<Preserved>,
}

/// Definitions and the instances placed from them, with overrides.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Library {
    definitions: BTreeMap<String, Definition>,
    instances: Vec<Instance>,
}

impl Library {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a definition from a selection, refusing a blank or duplicate name.
    pub fn extract(
        &mut self,
        name: &str,
        kind: NodeKind,
        properties: BTreeMap<PropertyName, PropertyValue>,
    ) -> Result<(), String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err("a definition name cannot be blank".to_owned());
        }
        if self.definitions.contains_key(trimmed) {
            return Err(format!("definition {trimmed:?} already exists"));
        }
        self.definitions.insert(
            trimmed.to_owned(),
            Definition {
                name: trimmed.to_owned(),
                version: 1,
                kind,
                properties,
            },
        );
        Ok(())
    }

    /// Places an instance, refusing an unknown definition and a duplicate instance identity.
    pub fn insert(&mut self, definition: &str, id: NodeId) -> Result<usize, String> {
        if !self.definitions.contains_key(definition) {
            return Err(format!("no definition {definition:?}"));
        }
        if self.instances.iter().any(|instance| instance.id == id) {
            return Err(format!("instance {} already exists", id.get()));
        }
        self.instances.push(Instance {
            definition: definition.to_owned(),
            id,
            overrides: BTreeMap::new(),
        });
        Ok(self.instances.len() - 1)
    }

    /// Records one explicit override on an instance, refusing an unknown instance.
    pub fn override_property(
        &mut self,
        id: NodeId,
        name: PropertyName,
        value: PropertyValue,
    ) -> Result<(), String> {
        let instance = self
            .instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .ok_or_else(|| format!("no instance {}", id.get()))?;
        instance.overrides.insert(name, value);
        Ok(())
    }

    /// Applies an edit to a definition and reports the instances it affected and the overrides it
    /// preserved.
    ///
    /// The instances themselves are not written to: an override is the instance's own word, so the
    /// definition change lands underneath it and the override still wins in [`Library::resolve`].
    pub fn edit(
        &mut self,
        name: &str,
        changed: BTreeMap<PropertyName, PropertyValue>,
    ) -> Result<Edit, String> {
        {
            let definition = self
                .definitions
                .get_mut(name)
                .ok_or_else(|| format!("no definition {name:?}"))?;
            for (property, value) in changed {
                definition.properties.insert(property, value);
            }
            definition.version += 1;
        }
        let affected: Vec<NodeId> = self
            .instances
            .iter()
            .filter(|instance| instance.definition == name)
            .map(|instance| instance.id)
            .collect();
        let preserved: Vec<Preserved> = self
            .instances
            .iter()
            .filter(|instance| instance.definition == name)
            .flat_map(|instance| {
                instance
                    .overrides
                    .iter()
                    .map(move |(property, value)| Preserved {
                        instance: instance.id,
                        name: property.clone(),
                        value: value.clone(),
                    })
            })
            .collect();
        Ok(Edit {
            affected,
            preserved,
        })
    }

    /// The values an instance resolves to: the definition's values, then its own overrides.
    pub fn resolve(&self, id: NodeId) -> Result<BTreeMap<PropertyName, PropertyValue>, String> {
        let instance = self
            .instances
            .iter()
            .find(|instance| instance.id == id)
            .ok_or_else(|| format!("no instance {}", id.get()))?;
        let definition = self
            .definitions
            .get(&instance.definition)
            .ok_or_else(|| format!("no definition {:?}", instance.definition))?;
        let mut properties = definition.properties.clone();
        for (name, value) in &instance.overrides {
            properties.insert(name.clone(), value.clone());
        }
        Ok(properties)
    }

    /// One definition, if it exists.
    pub fn definition(&self, name: &str) -> Option<&Definition> {
        self.definitions.get(name)
    }

    /// The definition names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.definitions.keys().map(String::as_str).collect()
    }

    /// One instance, if it exists.
    pub fn instance(&self, id: NodeId) -> Option<&Instance> {
        self.instances.iter().find(|instance| instance.id == id)
    }

    /// The identities of instances of a definition, in placement order.
    pub fn instances_of(&self, definition: &str) -> Vec<NodeId> {
        self.instances
            .iter()
            .filter(|instance| instance.definition == definition)
            .map(|instance| instance.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Library;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{NodeKind, PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn kind(text: &str) -> NodeKind {
        NodeKind::new(text).expect("a kind")
    }

    fn library_with_button() -> Library {
        let mut library = Library::new();
        let mut properties = BTreeMap::new();
        properties.insert(name("text"), PropertyValue::Text("Ok".to_owned()));
        library
            .extract("button", kind("button"), properties)
            .expect("an extraction");
        library
    }

    #[test]
    fn extract_refuses_a_blank_or_duplicate_name() {
        let mut library = library_with_button();
        assert!(
            library
                .extract("  ", kind("button"), BTreeMap::new())
                .is_err()
        );
        assert!(
            library
                .extract("button", kind("button"), BTreeMap::new())
                .is_err()
        );
    }

    #[test]
    fn insert_refuses_an_unknown_definition_and_a_duplicate_identity() {
        let mut library = library_with_button();
        assert!(library.insert("missing", NodeId::from_raw(1)).is_err());
        library
            .insert("button", NodeId::from_raw(1))
            .expect("a first");
        assert!(library.insert("button", NodeId::from_raw(1)).is_err());
    }

    #[test]
    fn a_definition_edit_does_not_clobber_a_legal_instance_override() {
        let mut library = library_with_button();
        library
            .insert("button", NodeId::from_raw(1))
            .expect("a placement");
        library
            .override_property(
                NodeId::from_raw(1),
                name("text"),
                PropertyValue::Text("Cancel".to_owned()),
            )
            .expect("an override");

        let mut changed = BTreeMap::new();
        changed.insert(name("text"), PropertyValue::Text("Submit".to_owned()));
        changed.insert(name("size"), PropertyValue::Integer(3));
        let edit = library.edit("button", changed).expect("an edit");

        let resolved = library.resolve(NodeId::from_raw(1)).expect("a resolution");
        assert_eq!(
            resolved.get(&name("text")),
            Some(&PropertyValue::Text("Cancel".to_owned())),
        );
        assert_eq!(
            resolved.get(&name("size")),
            Some(&PropertyValue::Integer(3))
        );
        assert_eq!(edit.preserved.len(), 1);
        assert_eq!(edit.preserved[0].name.as_str(), "text");
        assert_eq!(
            library
                .definition("button")
                .expect("a definition")
                .properties
                .get(&name("text")),
            Some(&PropertyValue::Text("Submit".to_owned())),
        );
    }

    #[test]
    fn a_definition_edit_names_the_affected_instances_and_preserved_overrides() {
        let mut library = library_with_button();
        library
            .insert("button", NodeId::from_raw(1))
            .expect("a placement");
        library
            .insert("button", NodeId::from_raw(2))
            .expect("a placement");
        library
            .override_property(
                NodeId::from_raw(2),
                name("text"),
                PropertyValue::Text("Cancel".to_owned()),
            )
            .expect("an override");

        let mut changed = BTreeMap::new();
        changed.insert(name("text"), PropertyValue::Text("Submit".to_owned()));
        let edit = library.edit("button", changed).expect("an edit");

        assert_eq!(
            edit.affected,
            vec![NodeId::from_raw(1), NodeId::from_raw(2)]
        );
        assert_eq!(edit.preserved.len(), 1);
        assert_eq!(edit.preserved[0].instance, NodeId::from_raw(2));
    }

    #[test]
    fn an_edit_of_an_unknown_definition_is_refused() {
        let mut library = library_with_button();
        assert!(library.edit("missing", BTreeMap::new()).is_err());
    }

    #[test]
    fn resolving_an_unknown_instance_is_refused() {
        let library = library_with_button();
        assert!(library.resolve(NodeId::from_raw(9)).is_err());
    }
}
