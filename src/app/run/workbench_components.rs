// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The component-library panel's rows (roadmap F12, `panel.components`).
//!
//! The rows come from the page document's `components` section (page format 4). The document types
//! are turned into the host's [`Registry`] and [`Variant`] only to reuse their checks: a nested
//! cycle, an unknown parameter, an undeclared slot and an unknown target are reported by the same
//! code the rest of the designer uses.
//!
//! **Boundary.** The page format stores definitions, instances and variants. Whether a variant's
//! target is the project's current target is not a document fact, so the panel checks that each
//! named target is one of the five profiles and says the project target itself is not read.

use crate::app::component::definition::{Definition, Instance, Parameter, Part, Registry, Slot};
use crate::app::component::variant::{self, Variant};
use crate::project::record::page::{PageComponents, PageDefinition, PagePart, PageVariant};

/// The rows for one page's component section.
pub(super) fn rows(source: &str, components: &PageComponents) -> Vec<String> {
    let mut rows = vec![format!("page: {source}")];
    let definitions = components.definitions();
    let instances = components.instances();
    let variants = components.variants();
    rows.push(format!(
        "definitions: {} · instances: {} · variants: {}",
        definitions.len(),
        instances.len(),
        variants.len()
    ));
    if definitions.is_empty() && instances.is_empty() && variants.is_empty() {
        rows.push("no component is declared in this page".to_owned());
        return rows;
    }

    let mut registry = Registry::new();
    for definition in definitions {
        rows.extend(definition_rows(definition));
        let model = model_definition(definition);
        if let Err(reason) = registry.insert(model) {
            rows.push(format!("definition {}: {reason}", definition.name()));
        }
    }
    for instance in instances {
        rows.push(format!(
            "instance {} of {}: {} argument(s), {} fill(s)",
            instance.node().get(),
            instance.definition(),
            instance.arguments().len(),
            instance.fills().len()
        ));
        for (name, value) in instance.arguments() {
            rows.push(format!("  argument {name} = {value:?}"));
        }
        for fill in instance.fills() {
            rows.push(format!(
                "  fill {}: {} part(s)",
                fill.slot(),
                fill.parts().len()
            ));
        }
        let model = Instance {
            definition: instance.definition().to_owned(),
            id: instance.node(),
            arguments: instance.arguments().clone(),
            fills: instance
                .fills()
                .iter()
                .map(|fill| {
                    (
                        fill.slot().to_owned(),
                        fill.parts().iter().map(model_part).collect(),
                    )
                })
                .collect(),
        };
        if let Err(reason) = registry.place(model) {
            rows.push(format!("  not placed: {reason}"));
        }
    }
    for variant in variants {
        rows.extend(variant_rows(variant));
    }
    rows
}

fn definition_rows(definition: &PageDefinition) -> Vec<String> {
    let mut rows = vec![format!(
        "definition {} v{}: {} parameter(s) · {} slot(s) · {} part(s)",
        definition.name(),
        definition.version(),
        definition.parameters().len(),
        definition.slots().len(),
        definition.body().len()
    )];
    for parameter in definition.parameters() {
        rows.push(match parameter.default() {
            Some(default) => format!("  parameter {} = {default:?}", parameter.name()),
            None => format!("  parameter {} (no default)", parameter.name()),
        });
    }
    for slot in definition.slots() {
        rows.push(format!("  slot {slot}"));
    }
    for part in definition.body() {
        rows.push(format!("  body: {}", part_text(part)));
    }
    rows
}

fn variant_rows(variant: &PageVariant) -> Vec<String> {
    let mut rows = vec![format!(
        "variant of {} ({}), kind {}: {} target(s) · {} override(s)",
        variant.instance().get(),
        variant.definition(),
        variant.kind().as_str(),
        variant.targets().len(),
        variant.overrides().len()
    )];
    let model = Variant {
        instance: variant.instance(),
        definition: variant.definition().to_owned(),
        kind: variant.kind().clone(),
        targets: variant.targets().to_vec(),
        overrides: variant.overrides().clone(),
    };
    if variant.targets().is_empty() {
        rows.push("  targets: (none); the variant is not applied to any target".to_owned());
    }
    for target in variant.targets() {
        if !variant::is_target(target) {
            rows.push(format!("  target {target}: not one of the five profiles"));
            continue;
        }
        rows.push(match variant::target_compatibility(&model, target) {
            Ok(()) => format!("  target {target}: accepted"),
            Err(reason) => format!("  target {target}: {reason}"),
        });
    }
    rows.push("  current target: not read from the project".to_owned());
    rows
}

fn part_text(part: &PagePart) -> String {
    match part {
        PagePart::Control { kind, properties } => {
            format!(
                "control {} ({} property(ies))",
                kind.as_str(),
                properties.len()
            )
        }
        PagePart::Nested(name) => format!("nested {name}"),
        PagePart::Slot(name) => format!("slot {name}"),
    }
}

fn model_part(part: &PagePart) -> Part {
    match part {
        PagePart::Control { kind, properties } => Part::Control {
            kind: kind.clone(),
            properties: properties.clone(),
        },
        PagePart::Nested(name) => Part::Nested {
            definition: name.clone(),
        },
        PagePart::Slot(name) => Part::Slot { name: name.clone() },
    }
}

fn model_definition(definition: &PageDefinition) -> Definition {
    Definition {
        name: definition.name().to_owned(),
        version: definition.version(),
        parameters: definition
            .parameters()
            .iter()
            .map(|parameter| Parameter {
                name: parameter.name().to_owned(),
                default: parameter.default().cloned(),
            })
            .collect(),
        slots: definition
            .slots()
            .iter()
            .map(|name| Slot { name: name.clone() })
            .collect(),
        body: definition.body().iter().map(model_part).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::contracts::identity::NodeId;
    use crate::contracts::model::NodeKind;
    use crate::project::record::page::{
        PageComponents, PageDefinition, PageInstance, PageParameter, PagePart, PageVariant,
    };

    #[test]
    fn an_empty_section_says_so() {
        let rows = rows("pages/main.json", &PageComponents::default());
        assert_eq!(rows[0], "page: pages/main.json");
        assert!(rows.iter().any(|row| row.contains("no component")));
    }

    #[test]
    fn definitions_instances_and_variants_are_rendered_and_checked() {
        let components = PageComponents::new(
            vec![
                PageDefinition::new(
                    "Card",
                    2,
                    vec![PageParameter::new(
                        "title",
                        Some(crate::contracts::model::PropertyValue::Text(
                            "Hi".to_owned(),
                        )),
                    )],
                    vec!["body".to_owned()],
                    vec![
                        PagePart::Control {
                            kind: NodeKind::new("panel").expect("a kind"),
                            properties: std::collections::BTreeMap::new(),
                        },
                        PagePart::Slot("body".to_owned()),
                    ],
                ),
                PageDefinition::new("Badge", 1, Vec::new(), Vec::new(), Vec::new()),
            ],
            vec![PageInstance::new("Card", NodeId::from_raw(2))],
            vec![PageVariant::new(
                NodeId::from_raw(2),
                "Card",
                NodeKind::new("panel").expect("a kind"),
                vec!["desktop".to_owned(), "unknown".to_owned()],
                std::collections::BTreeMap::new(),
            )],
        );
        let rows = rows("pages/main.json", &components);
        assert!(
            rows.iter()
                .any(|row| row.contains("definition Card v2: 1 parameter(s)")),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("instance 2 of Card")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("target desktop: accepted")),
            "{rows:?}"
        );
        assert!(
            rows.iter()
                .any(|row| row.contains("target unknown: not one of the five profiles")),
            "{rows:?}"
        );
    }

    #[test]
    fn a_nested_cycle_is_reported_by_the_registry() {
        let components = PageComponents::new(
            vec![
                PageDefinition::new(
                    "A",
                    1,
                    Vec::new(),
                    Vec::new(),
                    vec![PagePart::Nested("B".to_owned())],
                ),
                PageDefinition::new(
                    "B",
                    1,
                    Vec::new(),
                    Vec::new(),
                    vec![PagePart::Nested("A".to_owned())],
                ),
            ],
            Vec::new(),
            Vec::new(),
        );
        // The page reader refuses this before the panel sees it; the panel still reports the
        // registry's refusal if a caller hands it one, rather than drawing a cycle as a tree.
        let rows = rows("pages/main.json", &components);
        assert!(
            rows.iter().any(|row| row.contains("contains itself")),
            "{rows:?}"
        );
    }
}
