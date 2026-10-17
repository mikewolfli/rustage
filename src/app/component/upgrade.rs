// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Definition upgrade, cross-project paste, asset deletion, and the round-trip that must agree with
//! the static artifact (task T12.08).
//!
//! The acceptance line is *"the full round-trip and the static artifact share the same instance
//! semantics"*. The design keeps that true by resolving instances through one function:
//!
//! * **[`resolve`] is the only definition of what an instance means.** A definition's values, then
//!   the instance's overrides — nothing else. The static artifact is built by calling it, and the
//!   round-tripped project is resolved by calling it, so the two answers cannot be different by
//!   construction rather than by care.
//! * **The static artifact is pre-expanded, not re-derived.** [`StaticArtifact::expand`] stores the
//!   [`Resolved`] value for every instance, so comparing it with a live resolution is comparing two
//!   [`Resolved`] values, and a test asserts that equality directly.
//! * **Upgrade and paste keep overrides.** [`upgrade`] bumps a definition's version and reports the
//!   instances it affected and the overrides it preserved; it never writes to an instance.
//!   [`paste`] carries a definition across a project boundary and keeps the instance's overrides.
//! * **Deleting a referenced asset is refused.** [`delete_asset`] answers `Err` naming the instances
//!   that still reference it, because a silent delete would leave a dangling reference that only
//!   generation would notice.
//!
//! The serializer escapes the separators it uses and the parser rejects a malformed line, so the
//! round trip is a real parse of a real string rather than an identity on the in-memory value.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use crate::contracts::identity::NodeId;
use crate::contracts::model::{PropertyName, PropertyValue};

/// A definition inside a project.
#[derive(Clone, PartialEq, Debug)]
pub struct Definition {
    /// The definition's name.
    pub name: String,
    /// The definition's version.
    pub version: u32,
    /// The definition's values.
    pub properties: BTreeMap<PropertyName, PropertyValue>,
}

/// A placed instance inside a project.
#[derive(Clone, PartialEq, Debug)]
pub struct Instance {
    /// The instance's stable identity.
    pub id: NodeId,
    /// The definition the instance is of.
    pub definition: String,
    /// The instance's explicit overrides.
    pub overrides: BTreeMap<PropertyName, PropertyValue>,
}

/// A project's definitions, instances and referenced assets.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Project {
    /// The definitions, keyed by name.
    pub definitions: BTreeMap<String, Definition>,
    /// The placed instances.
    pub instances: Vec<Instance>,
    /// The logical paths of the assets the project holds.
    pub assets: BTreeSet<String>,
}

/// What an instance resolves to: its definition's identity, version and merged values.
#[derive(Clone, PartialEq, Debug)]
pub struct Resolved {
    /// The instance's identity.
    pub id: NodeId,
    /// The definition the instance is of.
    pub definition: String,
    /// The definition's version at resolution time.
    pub version: u32,
    /// The merged values: definition values, then the instance's overrides.
    pub properties: BTreeMap<PropertyName, PropertyValue>,
}

/// What an upgrade changed and what it preserved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Upgrade {
    /// The definition's name.
    pub name: String,
    /// The version before.
    pub from: u32,
    /// The version after.
    pub to: u32,
    /// The instances of the definition, in order.
    pub affected: Vec<NodeId>,
    /// The overrides on those instances, preserved untouched.
    pub preserved: Vec<Preserved>,
}

/// One override an upgrade preserved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Preserved {
    /// The instance that holds it.
    pub instance: NodeId,
    /// The overridden property.
    pub name: PropertyName,
}

/// A pre-expanded artifact: every instance already resolved.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct StaticArtifact {
    /// The resolved instances, in project order.
    pub instances: Vec<Resolved>,
}

impl StaticArtifact {
    /// Resolves every instance in a project, in project order.
    pub fn expand(project: &Project) -> Result<Self, String> {
        let mut instances = Vec::new();
        for instance in &project.instances {
            instances.push(resolve(project, instance.id)?);
        }
        Ok(Self { instances })
    }

    /// The resolved instance with a given identity, if it was expanded.
    pub fn get(&self, id: NodeId) -> Option<&Resolved> {
        self.instances.iter().find(|resolved| resolved.id == id)
    }
}

/// Resolves an instance to its definition, version and merged values.
pub fn resolve(project: &Project, id: NodeId) -> Result<Resolved, String> {
    let instance = project
        .instances
        .iter()
        .find(|instance| instance.id == id)
        .ok_or_else(|| format!("no instance {}", id.get()))?;
    let definition = project
        .definitions
        .get(&instance.definition)
        .ok_or_else(|| format!("no definition {:?}", instance.definition))?;
    let mut properties = definition.properties.clone();
    for (name, value) in &instance.overrides {
        properties.insert(name.clone(), value.clone());
    }
    Ok(Resolved {
        id,
        definition: definition.name.clone(),
        version: definition.version,
        properties,
    })
}

/// Bumps a definition's version and applies changes, reporting the instances it affected and the
/// overrides it preserved.
pub fn upgrade(
    project: &mut Project,
    name: &str,
    changed: &[(PropertyName, PropertyValue)],
) -> Result<Upgrade, String> {
    let definition = project
        .definitions
        .get_mut(name)
        .ok_or_else(|| format!("no definition {name:?}"))?;
    let from = definition.version;
    definition.version += 1;
    for (property, value) in changed {
        definition
            .properties
            .insert(property.clone(), value.clone());
    }
    let to = definition.version;
    let affected: Vec<NodeId> = project
        .instances
        .iter()
        .filter(|instance| instance.definition == name)
        .map(|instance| instance.id)
        .collect();
    let preserved: Vec<Preserved> = project
        .instances
        .iter()
        .filter(|instance| instance.definition == name)
        .flat_map(|instance| {
            instance.overrides.keys().map(move |property| Preserved {
                instance: instance.id,
                name: property.clone(),
            })
        })
        .collect();
    Ok(Upgrade {
        name: name.to_owned(),
        from,
        to,
        affected,
        preserved,
    })
}

/// Pastes an instance across a project boundary, bringing its definition along.
pub fn paste(
    project: &mut Project,
    source: &Instance,
    from: &Project,
    new_id: NodeId,
) -> Result<(), String> {
    if project
        .instances
        .iter()
        .any(|instance| instance.id == new_id)
    {
        return Err(format!("instance {} already exists", new_id.get()));
    }
    let definition = from
        .definitions
        .get(&source.definition)
        .ok_or_else(|| format!("no definition {:?}", source.definition))?;
    project
        .definitions
        .entry(definition.name.clone())
        .or_insert_with(|| definition.clone());
    project.instances.push(Instance {
        id: new_id,
        definition: source.definition.clone(),
        overrides: source.overrides.clone(),
    });
    Ok(())
}

/// Deletes an asset, refusing while an instance still references it.
pub fn delete_asset(project: &mut Project, path: &str) -> Result<(), String> {
    let referrers = asset_referrers(project, path);
    if !referrers.is_empty() {
        let names: Vec<String> = referrers.iter().map(|id| id.get().to_string()).collect();
        return Err(format!(
            "asset {path:?} is still referenced by instance(s) {}",
            names.join(", ")
        ));
    }
    if !project.assets.remove(path) {
        return Err(format!("no asset {path:?}"));
    }
    Ok(())
}

/// The instances whose values reference an asset path.
fn asset_referrers(project: &Project, path: &str) -> Vec<NodeId> {
    let references = |properties: &BTreeMap<PropertyName, PropertyValue>| {
        properties
            .values()
            .any(|value| matches!(value, PropertyValue::Asset(asset) if asset == path))
    };
    project
        .instances
        .iter()
        .filter(|instance| {
            references(&instance.overrides)
                || project
                    .definitions
                    .get(&instance.definition)
                    .is_some_and(|definition| references(&definition.properties))
        })
        .map(|instance| instance.id)
        .collect()
}

/// Writes a project as text: one definition, instance or asset per line.
pub fn serialize(project: &Project) -> String {
    let mut lines: Vec<String> = Vec::new();
    for definition in project.definitions.values() {
        lines.push(format!(
            "def\t{}\t{}\t{}",
            escape(&definition.name),
            definition.version,
            encode_properties(&definition.properties)
        ));
    }
    for instance in &project.instances {
        lines.push(format!(
            "inst\t{}\t{}\t{}",
            instance.id.get(),
            escape(&instance.definition),
            encode_properties(&instance.overrides)
        ));
    }
    for asset in &project.assets {
        lines.push(format!("asset\t{}", escape(asset)));
    }
    lines.join("\n")
}

/// Reads a project from text, refusing a malformed line.
///
/// A definition or instance line is split from the left only as far as its fixed fields: the
/// properties field is last and may itself contain real tabs (the separator between two property
/// pairs), so splitting the whole line on every tab would cut a multi-property map into pieces.
pub fn parse(text: &str) -> Result<Project, String> {
    let mut project = Project::default();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (tag, rest) = line.split_once('\t').unwrap_or((line, ""));
        match tag {
            "def" => {
                let (name, rest) = rest
                    .split_once('\t')
                    .ok_or_else(|| format!("malformed line {line:?}"))?;
                let (version, properties) = rest
                    .split_once('\t')
                    .ok_or_else(|| format!("malformed line {line:?}"))?;
                let name = unescape(name)
                    .ok_or_else(|| format!("definition name {name:?} is malformed"))?;
                let version = version
                    .parse::<u32>()
                    .map_err(|_| format!("{version:?} is not a version"))?;
                let properties = decode_properties(properties)
                    .ok_or_else(|| format!("definition {name:?} has malformed properties"))?;
                let previous = project.definitions.insert(
                    name.clone(),
                    Definition {
                        name,
                        version,
                        properties,
                    },
                );
                if previous.is_some() {
                    return Err("a definition appears twice".to_owned());
                }
            }
            "inst" => {
                let (id, rest) = rest
                    .split_once('\t')
                    .ok_or_else(|| format!("malformed line {line:?}"))?;
                let (definition, overrides) = rest
                    .split_once('\t')
                    .ok_or_else(|| format!("malformed line {line:?}"))?;
                let id = id
                    .parse::<u64>()
                    .map_err(|_| format!("{id:?} is not an instance id"))?;
                let definition = unescape(definition)
                    .ok_or_else(|| format!("definition name {definition:?} is malformed"))?;
                let overrides = decode_properties(overrides)
                    .ok_or_else(|| format!("instance {id:?} has malformed overrides"))?;
                project.instances.push(Instance {
                    id: NodeId::from_raw(id),
                    definition,
                    overrides,
                });
            }
            "asset" => {
                if rest.contains('\t') {
                    return Err(format!("malformed line {line:?}"));
                }
                let path =
                    unescape(rest).ok_or_else(|| format!("asset path {rest:?} is malformed"))?;
                project.assets.insert(path);
            }
            _ => return Err(format!("malformed line {line:?}")),
        }
    }
    Ok(project)
}

/// Escapes the separators the line format uses.
fn escape(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '|' => out.push_str("\\p"),
            other => out.push(other),
        }
    }
    out
}

/// Reverses [`escape`], or `None` on a malformed sequence.
fn unescape(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match characters.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'p' => out.push('|'),
            _ => return None,
        }
    }
    Some(out)
}

/// Encodes one property value.
fn encode_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Bool(flag) => format!("bool={flag}"),
        PropertyValue::Integer(number) => format!("int={number}"),
        PropertyValue::Number(number) => format!("num={number}"),
        PropertyValue::Text(text) => format!("text={}", escape(text)),
        PropertyValue::Variant(name) => format!("var={}", escape(name)),
        PropertyValue::Color(color) => format!("color={}", escape(color)),
        PropertyValue::Asset(asset) => format!("asset={}", escape(asset)),
    }
}

/// Decodes one property value, or `None` on a malformed tag.
fn decode_value(text: &str) -> Option<PropertyValue> {
    let (tag, rest) = text.split_once('=')?;
    match tag {
        "bool" => Some(PropertyValue::Bool(rest == "true")),
        "int" => rest.parse::<i64>().ok().map(PropertyValue::Integer),
        "num" => rest.parse::<f64>().ok().map(PropertyValue::Number),
        "text" => Some(PropertyValue::Text(unescape(rest)?)),
        "var" => Some(PropertyValue::Variant(unescape(rest)?)),
        "color" => Some(PropertyValue::Color(unescape(rest)?)),
        "asset" => Some(PropertyValue::Asset(unescape(rest)?)),
        _ => None,
    }
}

/// Encodes a property map as tab-separated `name|value` pairs.
fn encode_properties(properties: &BTreeMap<PropertyName, PropertyValue>) -> String {
    properties
        .iter()
        .map(|(name, value)| format!("{}|{}", escape(name.as_str()), encode_value(value)))
        .collect::<Vec<_>>()
        .join("\t")
}

/// Decodes a property map, or `None` on a malformed pair.
fn decode_properties(text: &str) -> Option<BTreeMap<PropertyName, PropertyValue>> {
    let mut properties = BTreeMap::new();
    if text.is_empty() {
        return Some(properties);
    }
    for pair in text.split('\t') {
        let (name, value) = pair.split_once('|')?;
        properties.insert(PropertyName::new(&unescape(name)?)?, decode_value(value)?);
    }
    Some(properties)
}

#[cfg(test)]
mod tests {
    use super::{Project, StaticArtifact, delete_asset, parse, paste, resolve, serialize, upgrade};
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::{PropertyName, PropertyValue};
    use std::collections::BTreeMap;

    fn name(text: &str) -> PropertyName {
        PropertyName::new(text).expect("a name")
    }

    fn project() -> Project {
        let mut project = Project::default();
        let mut properties = BTreeMap::new();
        properties.insert(
            name("text"),
            PropertyValue::Text("Ok|wide\there".to_owned()),
        );
        properties.insert(name("size"), PropertyValue::Integer(3));
        project.definitions.insert(
            "button".to_owned(),
            super::Definition {
                name: "button".to_owned(),
                version: 1,
                properties,
            },
        );
        let mut overrides = BTreeMap::new();
        overrides.insert(name("text"), PropertyValue::Text("Cancel".to_owned()));
        project.instances.push(super::Instance {
            id: NodeId::from_raw(1),
            definition: "button".to_owned(),
            overrides,
        });
        project.instances.push(super::Instance {
            id: NodeId::from_raw(2),
            definition: "button".to_owned(),
            overrides: BTreeMap::new(),
        });
        project.assets.insert("assets/logo.png".to_owned());
        project
    }

    #[test]
    fn a_serialized_project_round_trips() {
        let project = project();
        let text = serialize(&project);
        let parsed = parse(&text).expect("a parsed project");
        assert_eq!(parsed, project);
    }

    #[test]
    fn a_malformed_line_is_refused() {
        assert!(parse("nonsense").is_err());
        assert!(parse("def\tname\tnot-a-version\t").is_err());
    }

    #[test]
    fn the_round_trip_and_the_static_artifact_share_the_same_instance_semantics() {
        let project = project();
        let round_tripped = parse(&serialize(&project)).expect("a parsed project");
        let artifact = StaticArtifact::expand(&project).expect("a static artifact");

        for instance in &project.instances {
            let live = resolve(&round_tripped, instance.id).expect("a resolution");
            let static_value = artifact.get(instance.id).expect("a static instance");
            assert_eq!(&live, static_value);
            assert_eq!(live.definition, "button");
            assert_eq!(live.version, 1);
        }
    }

    #[test]
    fn upgrading_a_definition_keeps_every_instance_override() {
        let mut project = project();
        let changed = [(name("text"), PropertyValue::Text("Submit".to_owned()))];
        let upgrade = upgrade(&mut project, "button", &changed).expect("an upgrade");
        assert_eq!((upgrade.from, upgrade.to), (1, 2));
        assert_eq!(
            upgrade.affected,
            vec![NodeId::from_raw(1), NodeId::from_raw(2)]
        );
        assert_eq!(upgrade.preserved.len(), 1);

        let resolved = resolve(&project, NodeId::from_raw(1)).expect("a resolution");
        assert_eq!(resolved.version, 2);
        assert_eq!(
            resolved.properties.get(&name("text")),
            Some(&PropertyValue::Text("Cancel".to_owned())),
        );
    }

    #[test]
    fn an_unknown_definition_is_refused_by_upgrade() {
        let mut project = project();
        assert!(upgrade(&mut project, "missing", &[]).is_err());
    }

    #[test]
    fn pasting_an_instance_brings_its_definition_across_the_boundary() {
        let source = project();
        let mut target = Project::default();
        let instance = source.instances[0].clone();
        paste(&mut target, &instance, &source, NodeId::from_raw(50)).expect("a paste");

        assert!(target.definitions.contains_key("button"));
        let resolved = resolve(&target, NodeId::from_raw(50)).expect("a resolution");
        assert_eq!(resolved.definition, "button");
        assert_eq!(
            resolved.properties.get(&name("text")),
            Some(&PropertyValue::Text("Cancel".to_owned())),
        );
    }

    #[test]
    fn pasting_onto_a_taken_identity_is_refused() {
        let source = project();
        let mut target = Project::default();
        let instance = source.instances[0].clone();
        paste(&mut target, &instance, &source, NodeId::from_raw(50)).expect("a paste");
        assert!(paste(&mut target, &instance, &source, NodeId::from_raw(50)).is_err());
    }

    #[test]
    fn deleting_an_asset_an_instance_references_is_refused() {
        let mut project = project();
        let mut overrides = BTreeMap::new();
        overrides.insert(
            name("icon"),
            PropertyValue::Asset("assets/logo.png".to_owned()),
        );
        project.instances[0].overrides = overrides;
        let reason = delete_asset(&mut project, "assets/logo.png").expect_err("a reference");
        assert!(reason.contains("still referenced"), "{reason}");
        assert!(project.assets.contains("assets/logo.png"));
    }

    #[test]
    fn deleting_an_unreferenced_asset_removes_it() {
        let mut project = project();
        delete_asset(&mut project, "assets/logo.png").expect("a delete");
        assert!(!project.assets.contains("assets/logo.png"));
        assert!(delete_asset(&mut project, "assets/logo.png").is_err());
    }
}
