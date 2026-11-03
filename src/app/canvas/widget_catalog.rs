// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The widget catalog: the design palette's control kinds, captured from the library contract
//! (task T09.01).
//!
//! The acceptance line is *"the names come from the contract, and an unsupported item has a reason"*.
//! Both are about not inventing things, so the catalog invents nothing:
//!
//! * **Every name is a contract name.** [`Catalog::from_contract`] enumerates the pinned library's
//!   own registry ([`WidgetFactory::constructible_names`] and [`WidgetFactory::widget_names`]) and
//!   reads each control's designer manifest ([`designer_manifest`](fn@designer_manifest)). A [`WidgetDescriptor`] carries
//!   the canonical name the contract gave it; the catalog never synthesises one. A query for a kind
//!   the contract does not describe answers nothing rather than a plausible-looking entry.
//! * **An unsupported control says why.** [`Support`] is [`Supported`](Support::Supported),
//!   [`CrossProfile`](Support::CrossProfile) when the answer was measured under a different build
//!   profile and is therefore not conclusive, or [`Unsupported`](Support::Unsupported) with a
//!   reason. All three are read from the library's own
//!   [`availability_with`], never from a list kept here.
//! * **What the contract cannot express is listed, not dropped.** [`WidgetDescriptor::limits`] holds
//!   the parts of the contract that could not be read (for example a manifest whose property
//!   defaults are incomplete), with a note, so "not offered" is never mistaken for "does not exist".
//! * **The capture is dated and attributed.** [`Provenance`] records the library version and pinned
//!   revision, the capability-manifest protocol, the build profile/triple, the requested target
//!   profile and the capture time, so a palette can say which contract it describes.
//! * **Search is the one fuzzy rule.** [`Catalog::search`] reuses
//!   [`fuzzy_score`] over the id, name and category, so
//!   the palette and quick open rank the same way; favorites lift a kind but never hide the rest.
//!
//! [`Catalog::of`] remains the constructor for an explicitly supplied set of descriptors (the CLI's
//! existing test surface); it records no provenance and claims nothing about the library, which is
//! exactly what a hand-built catalog can honestly say. Production paths use
//! [`Catalog::from_contract`].

#![allow(dead_code)]

use std::collections::BTreeSet;

use rust_widgets::designer::generator::availability_with;
use rust_widgets::designer::{Availability, TargetProfile};
use rust_widgets::widget::WidgetFactory;
use rust_widgets::widget::capability::designer_manifest::{DesignerCommand, DesignerConstraint};
use rust_widgets::widget::capability::{
    CapabilityValue, DesignerEvent, DesignerManifest, DesignerProperty,
    EventSchema as ContractEventSchema, ManifestExportError,
    PropertySchema as ContractPropertySchema, PropertyValueKind, designer_manifest, shape_token,
};
use rust_widgets::widget::icon_source::{IconColorMode, IconSpec};

use crate::app::search::quick_open::fuzzy_score;
use crate::app::targets::snapshot::target_profile;

/// The catalog capture format this build writes.
///
/// A number of its own rather than the library version: the fields below can gain a member without
/// the library changing. A reader that stores a capture uses this to decide whether it understands
/// the capture at all.
pub const CATALOG_CAPTURE_VERSION: u32 = 1;

/// The type a property's value has.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ValueType {
    /// A number.
    Number,
    /// Free text.
    Text,
    /// A boolean.
    Bool,
    /// A colour.
    Color,
    /// A length (points, percent, `auto`).
    Dimension,
    /// One of a fixed set.
    Enum(Vec<String>),
    /// A reference to a resource of a kind.
    Resource {
        /// The resource kind (icon, image, font, ...).
        kind: String,
    },
    /// A rectangle in logical pixels.
    Rect,
    /// An icon source, built-in or a project resource.
    Icon,
    /// An ordered, id-keyed list of records.
    Records,
    /// An ordered list whose elements all have the named kind.
    List {
        /// The element kind (`text`, `number`, ...).
        element: String,
    },
}

impl ValueType {
    /// The type's name.
    pub fn name(&self) -> String {
        match self {
            Self::Number => "number".to_owned(),
            Self::Text => "text".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Color => "color".to_owned(),
            Self::Dimension => "dimension".to_owned(),
            Self::Enum(values) if values.is_empty() => "enum".to_owned(),
            Self::Enum(values) => format!("enum({})", values.join(",")),
            Self::Resource { kind } => format!("resource({kind})"),
            Self::Rect => "rect".to_owned(),
            Self::Icon => "icon".to_owned(),
            Self::Records => "records".to_owned(),
            Self::List { element } => format!("list({element})"),
        }
    }
}

/// One property a control exposes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PropertySchema {
    /// The property's name, from the contract.
    pub name: String,
    /// The value's type.
    pub value_type: ValueType,
    /// The default value, when the contract gives one.
    pub default: Option<String>,
    /// Whether the contract allows reading it.
    pub readable: bool,
    /// Whether the contract allows writing it.
    pub writable: bool,
    /// The accepted spellings of an enum property, in the order the contract gives them.
    ///
    /// The same list is already inside [`ValueType::Enum`]; it is carried separately so a reader
    /// does not have to match the type to answer "what may this property hold".
    pub tokens: Vec<String>,
    /// The runtime-enforced constraints, rendered as the contract publishes them.
    ///
    /// Rendered rather than stored as `f64` so [`PropertySchema`] keeps `Eq`; the structured form
    /// is what the Inspector reads from the manifest itself (blue22 H-12), and a palette only has
    /// to display it.
    pub constraints: Vec<String>,
}

impl PropertySchema {
    /// A property.
    pub fn new(name: impl Into<String>, value_type: ValueType) -> Self {
        Self {
            name: name.into(),
            value_type,
            default: None,
            readable: true,
            writable: true,
            tokens: Vec::new(),
            constraints: Vec::new(),
        }
    }

    /// The same property with a default.
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// The same property with the contract's read/write access.
    pub fn with_access(mut self, readable: bool, writable: bool) -> Self {
        self.readable = readable;
        self.writable = writable;
        self
    }

    /// The same property with its accepted enum spellings.
    pub fn with_tokens(mut self, tokens: impl IntoIterator<Item = String>) -> Self {
        self.tokens = tokens.into_iter().collect();
        self
    }

    /// The same property with one rendered constraint.
    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraints.push(constraint.into());
        self
    }

    /// The property in one line.
    pub fn render(&self) -> String {
        let mut line = match &self.default {
            Some(default) => format!("{}: {} = {default}", self.name, self.value_type.name()),
            None => format!("{}: {}", self.name, self.value_type.name()),
        };
        // Enum tokens already appear inside `value_type.name()`; printing them again would make
        // one fact look like two. The separate `tokens` field is for programmatic consumers.
        match (self.readable, self.writable) {
            (false, true) => line.push_str(" (write-only)"),
            (true, false) => line.push_str(" (read-only)"),
            (false, false) => line.push_str(" (no access)"),
            (true, true) => {}
        }
        for constraint in &self.constraints {
            line.push_str(&format!(" {{{constraint}}}"));
        }
        line
    }
}

/// One event a control publishes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EventSchema {
    /// The published name, as `connect_event` accepts it.
    pub name: String,
    /// The declared payload type, or `None` when the event carries no value.
    pub payload: Option<ValueType>,
    /// The payload's arrangement (`scalar`, `list`, `tuple2`, ...), from the contract's own token.
    pub shape: Option<String>,
}

impl EventSchema {
    /// The event in one line.
    pub fn render(&self) -> String {
        match (&self.payload, &self.shape) {
            (Some(payload), Some(shape)) => {
                format!("{}: {} ({shape})", self.name, payload.name())
            }
            (Some(payload), None) => format!("{}: {}", self.name, payload.name()),
            (None, _) => format!("{}: no payload", self.name),
        }
    }
}

/// One command (method) a control accepts.
///
/// The manifest publishes these as BLUE26 B26.03; a palette that showed only properties and events
/// would leave out the calls a designer can make without writing Rust.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommandSchema {
    /// The published command name.
    pub name: String,
    /// The parameter shape: `none`, `value` or `composite`.
    pub signature: String,
    /// The parameter name, empty for a payload-free command.
    pub parameter: String,
    /// The declared kind of a `value` parameter.
    pub kind: Option<ValueType>,
    /// How many values a `composite` payload carries, or `None` for a list.
    pub arity: Option<usize>,
    /// The declared kind of each composite element, when homogeneous.
    pub element: Option<ValueType>,
    /// The side-effect class the contract declares.
    pub effect: String,
}

impl CommandSchema {
    /// The command in one line.
    pub fn render(&self) -> String {
        let mut line = format!("{}: {}", self.name, self.signature);
        if !self.parameter.is_empty() {
            line.push_str(&format!(" {}", self.parameter));
            if let Some(kind) = &self.kind {
                line.push_str(&format!(": {}", kind.name()));
            }
        }
        if let Some(arity) = self.arity {
            line.push_str(&format!(" x{arity}"));
        }
        if let Some(element) = &self.element {
            line.push_str(&format!(" of {}", element.name()));
        }
        if !self.effect.is_empty() {
            line.push_str(&format!(" ({})", self.effect));
        }
        line
    }
}

/// Whether the current target can place a control.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Support {
    /// It can be placed, and the answer was measured under the target profile.
    Supported,
    /// The name resolves, but the build's profile differs from the target, so the answer is not
    /// conclusive for that target. The library permits placement and records the caveat.
    CrossProfile {
        /// Why the answer is not conclusive.
        reason: String,
    },
    /// It cannot be placed, for a named reason.
    Unsupported {
        /// Why.
        reason: String,
    },
}

impl Support {
    /// Reads the library's availability verdict as support.
    ///
    /// The four verdicts stay four facts: a caveat is not promoted to a support claim, and
    /// `Unavailable` stays distinct from `Unknown` because the library distinguishes "the contract
    /// knows this name but this build cannot construct it" from "nothing names it".
    pub fn of(availability: Availability, profile: &str) -> Self {
        match availability {
            Availability::Local => Self::Supported,
            Availability::CrossProfileCaveat => Self::CrossProfile {
                reason: format!(
                    "resolved in this build, whose profile differs from the target {profile}, so the answer is not conclusive for {profile}"
                ),
            },
            Availability::Unavailable => Self::Unsupported {
                reason: format!(
                    "the contract knows this control but this build has no constructor for it, so {profile} cannot place it"
                ),
            },
            Availability::Unknown => Self::Unsupported {
                reason: "no capability and no constructor is registered for this name".to_owned(),
            },
        }
    }

    /// Asks the library about one name on one profile.
    ///
    /// This is the same probe [`Catalog::from_contract`] uses. It accepts a name the registry does
    /// not know, and answers [`Support::Unsupported`] with the library's `Unknown` reason, so a
    /// caller can explain a refusal instead of guessing whether a name exists.
    pub fn query(
        factory: &WidgetFactory,
        name: &str,
        profile: &str,
    ) -> Result<Self, ContractError> {
        let target = profile_target(profile)?;
        Ok(Self::of(availability_with(factory, name, target), profile))
    }

    /// Whether the target can place it.
    ///
    /// A cross-profile caveat counts as placeable — the library permits generation and attaches a
    /// warning — but [`Self::is_verified`] does not.
    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unsupported { .. })
    }

    /// Whether the answer was measured under the target profile, rather than a different one.
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Supported)
    }

    /// The reason, when there is one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::CrossProfile { reason } | Self::Unsupported { reason } => Some(reason),
        }
    }
}

/// One control kind, as the contract describes it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WidgetDescriptor {
    /// The contract's id (its canonical name).
    pub id: String,
    /// The display name. The contract publishes no separate display name, so this is the canonical
    /// name rather than a prettification invented here.
    pub name: String,
    /// The category it is grouped under.
    ///
    /// The library publishes no designer category; the contract's own [`WidgetKind`] spelling is
    /// used rather than inventing a taxonomy. A real category table is a later task, not a guess
    /// made in this one.
    ///
    /// [`WidgetKind`]: rust_widgets::widget::WidgetKind
    pub category: String,
    /// The properties it exposes.
    pub properties: Vec<PropertySchema>,
    /// The events it publishes.
    pub events: Vec<EventSchema>,
    /// The commands (methods) it accepts.
    pub commands: Vec<CommandSchema>,
    /// The canonical names it accepts as children, or [`ANY_CHILD`] for any constructible child.
    ///
    /// Empty for a leaf. The distinction between "no children" and "any child" is the wildcard,
    /// not an empty list.
    pub children: Vec<String>,
    /// Every other name the factory accepts for this control.
    pub aliases: Vec<String>,
    /// The canonical control this name is a second spelling of, empty for a canonical row.
    pub alias_of: String,
    /// Whether the target can place it.
    pub support: Support,
    /// What the contract cannot express, with a note.
    pub limits: Vec<String>,
}

/// The manifest's "any constructible child" token (upstream `ANY_CHILD_WILDCARD`).
///
/// Spelled here rather than imported so the catalog's own type is not silently tied to an internal
/// constant; a test pins it against the library's exported value.
pub const ANY_CHILD: &str = "*";

impl WidgetDescriptor {
    /// A supported control.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        category: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            category: category.into(),
            properties: Vec::new(),
            events: Vec::new(),
            commands: Vec::new(),
            children: Vec::new(),
            aliases: Vec::new(),
            alias_of: String::new(),
            support: Support::Supported,
            limits: Vec::new(),
        }
    }

    /// The same descriptor, unsupported for a reason.
    pub fn unsupported(mut self, reason: impl Into<String>) -> Self {
        self.support = Support::Unsupported {
            reason: reason.into(),
        };
        self
    }

    /// The same descriptor with a property.
    pub fn with_property(mut self, property: PropertySchema) -> Self {
        self.properties.push(property);
        self
    }

    /// The same descriptor with an event.
    pub fn with_event(mut self, event: EventSchema) -> Self {
        self.events.push(event);
        self
    }

    /// The same descriptor with a command.
    pub fn with_command(mut self, command: CommandSchema) -> Self {
        self.commands.push(command);
        self
    }

    /// The same descriptor with a child kind.
    pub fn with_child(mut self, child: impl Into<String>) -> Self {
        self.children.push(child.into());
        self
    }

    /// The same descriptor marked as a second spelling of `canonical`.
    pub fn with_alias_of(mut self, canonical: impl Into<String>) -> Self {
        self.alias_of = canonical.into();
        self
    }

    /// The same descriptor with a limit note.
    pub fn with_limit(mut self, note: impl Into<String>) -> Self {
        self.limits.push(note.into());
        self
    }

    /// Whether the contract lets this control hold `child`.
    ///
    /// A leaf answers `false`; a container that accepts any constructible child answers `true` for
    /// every name; a constrained container answers only for a listed name. This is the rule the
    /// palette needs for a drop target, read from the contract rather than guessed from a name.
    pub fn accepts_child(&self, child: &str) -> bool {
        self.children
            .iter()
            .any(|allowed| allowed == ANY_CHILD || allowed == child)
    }

    /// Whether the contract declares this control as a container.
    pub fn is_container(&self) -> bool {
        !self.children.is_empty()
    }

    /// The descriptor in one line.
    pub fn render(&self) -> String {
        let mut line = format!(
            "{} [{}] — properties: {}, events: {}, commands: {}, children: {}",
            self.name,
            self.category,
            self.properties.len(),
            self.events.len(),
            self.commands.len(),
            self.children.len()
        );
        if !self.alias_of.is_empty() {
            line.push_str(&format!(" — alias of {}", self.alias_of));
        }
        match &self.support {
            Support::Supported => {}
            Support::CrossProfile { reason } => line.push_str(&format!(" — caveat: {reason}")),
            Support::Unsupported { reason } => line.push_str(&format!(" — unsupported: {reason}")),
        }
        line
    }
}

/// Where a captured catalog came from.
///
/// Every field is a fact the library or the host can be asked for: the version and protocol come
/// from the designer manifest, the revision from the pinned-dependency constants, the profiles and
/// triple from the build, and the capture time from the clock at the moment of capture.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Provenance {
    /// The dependency crate the contract was read from.
    pub crate_name: String,
    /// The library version the manifest reports.
    pub version: String,
    /// The pinned revision of that dependency, from the host's probe constants.
    pub revision: String,
    /// The capability-manifest protocol the descriptors were read in (`0` when no manifest could
    /// be read).
    pub protocol: u32,
    /// The profile the library was built under.
    pub build_profile: String,
    /// The target triple the library was built for.
    pub triple: String,
    /// The profile the catalog was asked about.
    pub target_profile: String,
    /// When the capture ran, as seconds since the Unix epoch (`"unknown"` if the clock is before
    /// the epoch).
    pub captured_at: String,
    /// The catalog capture format this build writes.
    pub capture_version: u32,
}

impl Provenance {
    /// The provenance in one line.
    pub fn render(&self) -> String {
        format!(
            "contract: {} {} (revision {}), manifest protocol {}, built for {} {}, target {}, captured {} (capture v{})",
            self.crate_name,
            self.version,
            self.revision,
            self.protocol,
            self.build_profile,
            self.triple,
            self.target_profile,
            self.captured_at,
            self.capture_version
        )
    }
}

/// Why a catalog could not be captured from the contract.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ContractError {
    /// The profile name is not one the library's target declares.
    UnknownProfile(String),
}

impl ContractError {
    /// The refusal in one line.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownProfile(profile) => format!(
                "{profile:?} is not a profile the library's target declares (desktop, tablet, mobile, mini, embedded)"
            ),
        }
    }
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

/// The catalog of control kinds the contract describes.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Catalog {
    descriptors: Vec<WidgetDescriptor>,
    favorites: BTreeSet<String>,
    provenance: Option<Provenance>,
}

impl Catalog {
    /// A catalog over explicitly supplied descriptors.
    ///
    /// This is the CLI's existing surface: it describes exactly what it was handed and records no
    /// provenance, because a hand-built list has none to record. Production paths use
    /// [`Catalog::from_contract`].
    pub fn of(descriptors: Vec<WidgetDescriptor>) -> Self {
        Self {
            descriptors,
            favorites: BTreeSet::new(),
            provenance: None,
        }
    }

    /// Captures the catalog from the pinned library contract for `profile`.
    ///
    /// The names are the factory's own registry, not a list in this file: every constructible
    /// canonical name, plus canonical names the capability table knows but this build cannot
    /// construct (so an unavailable control is reported rather than silently absent). Each
    /// control's properties and events come from its designer manifest; a manifest that cannot be
    /// read leaves the descriptor's schema empty and says why in its limits.
    pub fn from_contract(factory: &WidgetFactory, profile: &str) -> Result<Self, ContractError> {
        Self::from_contract_at(factory, profile, captured_now())
    }

    /// Captures the catalog from the host's own factory for `profile`.
    ///
    /// The host factory is the pinned library's defaults **plus** the suites installed by
    /// [`crate::app::contract::factory`], so a suite control such as `form` or `form_set` is in
    /// the catalog with the same schema the parent crate's controls get. Reaching for
    /// [`WidgetFactory::shared`] here would capture the parent crate's table only, where those
    /// names do not exist.
    pub fn from_host_contract(profile: &str) -> Result<Self, ContractError> {
        Self::from_contract(crate::app::contract::factory(), profile)
    }

    /// [`Catalog::from_contract`] with a caller-supplied capture time.
    ///
    /// The clock is a parameter so a stored or replayed capture can name the moment it was taken
    /// instead of the moment it was re-read.
    pub fn from_contract_at(
        factory: &WidgetFactory,
        profile: &str,
        captured_at: impl Into<String>,
    ) -> Result<Self, ContractError> {
        // Refuse an unknown profile before reading anything: an answer about a profile the library
        // does not declare would be a guess dressed as a capability.
        profile_target(profile)?;

        // The constructible set is what a generator must be able to name; the registry set adds
        // canonical rows that exist but cannot be constructed under this build, which the catalog
        // reports as unavailable instead of dropping.
        let mut names: Vec<String> = factory
            .constructible_names()
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        let constructible: BTreeSet<String> = names.iter().cloned().collect();
        for name in factory.widget_names() {
            if !constructible.contains(name) {
                names.push(name.to_owned());
            }
        }
        names.sort();
        names.dedup();

        let mut header: Option<DesignerManifest> = None;
        let mut descriptors = Vec::with_capacity(names.len());
        for name in &names {
            let manifest = designer_manifest(factory, name);
            if header.is_none()
                && let Ok(manifest) = &manifest
            {
                header = Some(manifest.clone());
            }
            descriptors.push(descriptor_from_contract(factory, name, profile, manifest));
        }

        let provenance = Provenance {
            crate_name: crate::probe::DEPENDENCY_CRATE.to_owned(),
            version: header
                .as_ref()
                .map(|manifest| manifest.library.clone())
                .unwrap_or_else(|| crate::probe::DEPENDENCY_VERSION.to_owned()),
            revision: crate::probe::DEPENDENCY_REVISION.to_owned(),
            protocol: header
                .as_ref()
                .map(|manifest| manifest.protocol)
                .unwrap_or(0),
            build_profile: header
                .as_ref()
                .map(|manifest| manifest.profile.clone())
                .unwrap_or_default(),
            triple: header
                .as_ref()
                .map(|manifest| manifest.triple.clone())
                .unwrap_or_default(),
            target_profile: profile.to_owned(),
            captured_at: captured_at.into(),
            capture_version: CATALOG_CAPTURE_VERSION,
        };

        Ok(Self {
            descriptors,
            favorites: BTreeSet::new(),
            provenance: Some(provenance),
        })
    }

    /// Where a captured catalog came from, when it was captured from the contract.
    pub fn provenance(&self) -> Option<&Provenance> {
        self.provenance.as_ref()
    }

    /// How many kinds it describes.
    pub fn len(&self) -> usize {
        self.descriptors.len()
    }

    /// Every captured descriptor, in contract order.
    pub fn descriptors(&self) -> &[WidgetDescriptor] {
        &self.descriptors
    }

    /// Whether it describes none.
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }

    /// A descriptor by its contract id.
    pub fn get(&self, id: &str) -> Option<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.id == id)
    }

    /// The categories, in order.
    pub fn categories(&self) -> BTreeSet<&str> {
        self.descriptors
            .iter()
            .map(|descriptor| descriptor.category.as_str())
            .collect()
    }

    /// The kinds in a category.
    pub fn in_category(&self, category: &str) -> Vec<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.category == category)
            .collect()
    }

    /// Searches by a query, best first, lifting favorites.
    ///
    /// Only what the catalog describes can appear: there is no entry to fall back to.
    pub fn search(&self, query: &str) -> Vec<&WidgetDescriptor> {
        let mut hits: Vec<(i64, bool, &WidgetDescriptor)> = self
            .descriptors
            .iter()
            .filter_map(|descriptor| {
                let haystack = format!(
                    "{} {} {}",
                    descriptor.id, descriptor.name, descriptor.category
                );
                fuzzy_score(query, &haystack).map(|score| {
                    let favorite = self.favorites.contains(&descriptor.id);
                    (score, favorite, descriptor)
                })
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.id.cmp(&b.2.id)));
        hits.into_iter()
            .map(|(_, _, descriptor)| descriptor)
            .collect()
    }

    /// Marks a kind a favorite, or not.
    pub fn favorite(&mut self, id: &str, on: bool) {
        if on {
            self.favorites.insert(id.to_owned());
        } else {
            self.favorites.remove(id);
        }
    }

    /// Whether a kind is a favorite.
    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.contains(id)
    }

    /// Adds an explanatory note to a kind, or answers `false` when it is not in the catalog.
    pub fn annotate(&mut self, id: &str, note: impl Into<String>) -> bool {
        match self
            .descriptors
            .iter_mut()
            .find(|descriptor| descriptor.id == id)
        {
            Some(descriptor) => {
                descriptor.limits.push(note.into());
                true
            }
            None => false,
        }
    }

    /// The unsupported kinds, each with its reason.
    pub fn unsupported(&self) -> Vec<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| matches!(descriptor.support, Support::Unsupported { .. }))
            .collect()
    }

    /// The kinds whose support answer was measured under a different build profile.
    pub fn caveats(&self) -> Vec<&WidgetDescriptor> {
        self.descriptors
            .iter()
            .filter(|descriptor| matches!(descriptor.support, Support::CrossProfile { .. }))
            .collect()
    }

    /// Why a kind cannot be placed, when it cannot, or why its placement is not verified.
    pub fn reason(&self, id: &str) -> Option<&str> {
        self.get(id)
            .and_then(|descriptor| descriptor.support.reason())
    }

    /// A kind's property schema.
    pub fn schema(&self, id: &str) -> Option<&[PropertySchema]> {
        self.get(id)
            .map(|descriptor| descriptor.properties.as_slice())
    }

    /// A kind's event schema.
    pub fn events(&self, id: &str) -> Option<&[EventSchema]> {
        self.get(id).map(|descriptor| descriptor.events.as_slice())
    }

    /// A kind's command schema.
    pub fn commands(&self, id: &str) -> Option<&[CommandSchema]> {
        self.get(id)
            .map(|descriptor| descriptor.commands.as_slice())
    }

    /// The canonical names a kind accepts as children, or `["*"]` for any constructible child.
    pub fn children(&self, id: &str) -> Option<&[String]> {
        self.get(id)
            .map(|descriptor| descriptor.children.as_slice())
    }

    /// What the captured descriptors do and do not carry.
    ///
    /// This is the completeness check a palette needs before it claims a control is fully
    /// described: a row with no properties, events or commands is listed rather than silently
    /// rendered as an empty tile, and a container states its child rule. It makes no judgement —
    /// a label legitimately publishes no command — it only measures what the contract answered.
    pub fn audit(&self) -> Vec<String> {
        let mut lines = vec![format!("audit: {} kind(s)", self.descriptors.len())];

        let unread: Vec<&str> = self
            .descriptors
            .iter()
            .filter(|descriptor| {
                descriptor
                    .limits
                    .iter()
                    .any(|limit| limit.contains("manifest could not be read"))
            })
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        lines.push(format!(
            "manifests: {} read, {} unread{}",
            self.descriptors.len() - unread.len(),
            unread.len(),
            if unread.is_empty() {
                String::new()
            } else {
                format!(": {}", unread.join(", "))
            }
        ));

        for (label, count) in [
            (
                "properties",
                self.descriptors
                    .iter()
                    .filter(|descriptor| !descriptor.properties.is_empty())
                    .count(),
            ),
            (
                "events",
                self.descriptors
                    .iter()
                    .filter(|descriptor| !descriptor.events.is_empty())
                    .count(),
            ),
            (
                "commands",
                self.descriptors
                    .iter()
                    .filter(|descriptor| !descriptor.commands.is_empty())
                    .count(),
            ),
            (
                "containers",
                self.descriptors
                    .iter()
                    .filter(|descriptor| descriptor.is_container())
                    .count(),
            ),
            (
                "alias rows",
                self.descriptors
                    .iter()
                    .filter(|descriptor| !descriptor.alias_of.is_empty())
                    .count(),
            ),
        ] {
            lines.push(format!("{label}: {count}"));
        }

        for descriptor in &self.descriptors {
            if !descriptor.alias_of.is_empty() {
                lines.push(format!(
                    "  {}: alias of {}",
                    descriptor.id, descriptor.alias_of
                ));
            }
            if descriptor.is_container() {
                let rule = if descriptor.children.iter().any(|child| child == ANY_CHILD) {
                    "any constructible child"
                } else {
                    "listed children"
                };
                lines.push(format!(
                    "  {}: {rule} ({})",
                    descriptor.id,
                    descriptor.children.join(", ")
                ));
                continue;
            }
            if descriptor.properties.is_empty() {
                lines.push(format!("  {}: no declared property", descriptor.id));
            }
            if descriptor.events.is_empty() {
                lines.push(format!("  {}: no declared event", descriptor.id));
            }
            if descriptor.commands.is_empty() {
                lines.push(format!("  {}: no declared command", descriptor.id));
            }
        }
        lines
    }

    /// The audit failures that make the catalog unusable.
    ///
    /// The baseline is deliberately narrow: a control with no events or no commands is not a
    /// failure (a presentation-only control is legitimate), but a manifest that could not be read,
    /// a child kind the catalog does not contain, or an alias target it does not contain are facts
    /// that make the table lie. Those are the only problems this gate reports.
    pub fn audit_problems(&self) -> Vec<String> {
        let known: BTreeSet<&str> = self
            .descriptors
            .iter()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        let mut problems = Vec::new();
        for descriptor in &self.descriptors {
            if descriptor
                .limits
                .iter()
                .any(|limit| limit.contains("manifest could not be read"))
            {
                problems.push(format!(
                    "{}: the designer manifest could not be read",
                    descriptor.id
                ));
            }
            for child in &descriptor.children {
                if child != ANY_CHILD && !known.contains(child.as_str()) {
                    problems.push(format!(
                        "{}: child kind {child:?} is not in the catalog",
                        descriptor.id
                    ));
                }
            }
            if !descriptor.alias_of.is_empty() && !known.contains(descriptor.alias_of.as_str()) {
                problems.push(format!(
                    "{}: alias target {:?} is not in the catalog",
                    descriptor.id, descriptor.alias_of
                ));
            }
        }
        problems
    }

    /// The catalog in one report.
    pub fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(provenance) = &self.provenance {
            lines.push(provenance.render());
        }
        lines.push(format!(
            "catalog: {} kind(s), {} unsupported, {} caveat(s), {} favorite(s)",
            self.descriptors.len(),
            self.unsupported().len(),
            self.caveats().len(),
            self.favorites.len()
        ));
        for descriptor in &self.descriptors {
            lines.push(format!("  {}", descriptor.render()));
        }
        lines
    }
}

/// The profile name as the library's target, or the refusal.
fn profile_target(profile: &str) -> Result<TargetProfile, ContractError> {
    target_profile(profile).ok_or_else(|| ContractError::UnknownProfile(profile.to_owned()))
}

/// The capture time, as seconds since the Unix epoch.
///
/// A clock that cannot be read stays a stated "unknown" rather than an invented number.
fn captured_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_else(|_| "unknown".to_owned())
}

/// One descriptor, from the manifest when it can be read and from the capability schema otherwise.
fn descriptor_from_contract(
    factory: &WidgetFactory,
    name: &str,
    profile: &str,
    manifest: Result<DesignerManifest, ManifestExportError>,
) -> WidgetDescriptor {
    let capability = factory.capability(name);
    // Protocol 5 publishes the designer-palette category; a manifest that predates it leaves the
    // field empty, and the contract's kind spelling is the one grouping it does declare. An
    // unregistered name has neither, and an empty category says so.
    let kind_category = capability
        .map(|capability| capability.kind.css_name().to_owned())
        .unwrap_or_default();

    let mut limits = Vec::new();
    let (category, properties, events, commands, children, aliases, alias_of) = match manifest {
        Ok(manifest) => {
            let category = if manifest.category.is_empty() {
                kind_category
            } else {
                manifest.category.clone()
            };
            (
                category,
                manifest
                    .properties
                    .iter()
                    .map(property_from_designer)
                    .collect(),
                manifest.events.iter().map(event_from_designer).collect(),
                manifest
                    .commands
                    .iter()
                    .map(command_from_designer)
                    .collect(),
                manifest.children.clone(),
                manifest.aliases.clone(),
                manifest.alias_of.clone(),
            )
        }
        Err(reason) => {
            // The manifest is the only source that carries defaults; falling back keeps the
            // declared properties and events visible while the note records what is missing.
            limits.push(format!(
                "the designer manifest could not be read ({reason}); properties and events are the capability schema, so no defaults, commands or child kinds are shown"
            ));
            match capability {
                Some(capability) => (
                    kind_category,
                    capability
                        .properties
                        .iter()
                        .map(property_from_schema)
                        .collect(),
                    capability.events.iter().map(event_from_schema).collect(),
                    Vec::new(),
                    Vec::new(),
                    capability
                        .aliases
                        .iter()
                        .map(|alias| (*alias).to_owned())
                        .collect(),
                    String::new(),
                ),
                None => (
                    kind_category,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    String::new(),
                ),
            }
        }
    };

    let support =
        Support::query(factory, name, profile).unwrap_or_else(|refusal| Support::Unsupported {
            reason: refusal.message(),
        });

    WidgetDescriptor {
        id: name.to_owned(),
        // The contract's display name is its canonical name; nothing is prettified here.
        name: name.to_owned(),
        category,
        properties,
        events,
        commands,
        children,
        aliases,
        alias_of,
        support,
        limits,
    }
}

/// One property from the designer manifest.
fn property_from_designer(property: &DesignerProperty) -> PropertySchema {
    PropertySchema {
        name: property.name.clone(),
        value_type: value_type(property.kind, &property.tokens),
        default: render_default(&property.default_value),
        readable: property.readable,
        writable: property.writable,
        tokens: property.tokens.clone(),
        constraints: property.constraints.iter().map(render_constraint).collect(),
    }
}

/// One property from the capability schema, when the manifest could not supply defaults.
fn property_from_schema(schema: &ContractPropertySchema) -> PropertySchema {
    let tokens: Vec<String> = schema
        .accepted_tokens
        .iter()
        .map(|token| (*token).to_owned())
        .collect();
    PropertySchema {
        name: schema.name.to_owned(),
        value_type: value_type(schema.value_kind, &tokens),
        default: None,
        readable: schema.readable,
        writable: schema.writable,
        tokens,
        constraints: schema
            .constraints
            .iter()
            .map(|constraint| constraint_token(*constraint))
            .collect(),
    }
}

/// One command from the designer manifest.
fn command_from_designer(command: &DesignerCommand) -> CommandSchema {
    CommandSchema {
        name: command.name.clone(),
        signature: command.signature.to_owned(),
        parameter: command.parameter.clone(),
        kind: command.kind.map(|kind| value_type(kind, &[])),
        arity: command.arity,
        element: command.element.map(|kind| value_type(kind, &[])),
        effect: command.effect.to_owned(),
    }
}

/// One runtime constraint in the manifest's display spelling.
fn render_constraint(constraint: &DesignerConstraint) -> String {
    match constraint.kind {
        "range" => {
            let minimum = constraint
                .minimum
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-inf".to_owned());
            let maximum = constraint
                .maximum
                .map(|value| value.to_string())
                .unwrap_or_else(|| "inf".to_owned());
            match constraint.step {
                Some(step) => format!("range {minimum}..={maximum} step {step}"),
                None => format!("range {minimum}..={maximum}"),
            }
        }
        "precision" => constraint
            .places
            .map(|places| format!("precision {places}"))
            .unwrap_or_else(|| "precision".to_owned()),
        "required" => "required".to_owned(),
        other => other.to_owned(),
    }
}

/// A runtime constraint's display spelling, from the capability schema.
fn constraint_token(constraint: rust_widgets::widget::capability::Constraint) -> String {
    use rust_widgets::widget::capability::Constraint;
    match constraint {
        Constraint::Range(range) => match range.step {
            Some(step) => format!("range {}..={} step {step}", range.minimum, range.maximum),
            None => format!("range {}..={}", range.minimum, range.maximum),
        },
        Constraint::Precision { places } => places
            .map(|places| format!("precision {places}"))
            .unwrap_or_else(|| "precision".to_owned()),
        Constraint::Required => "required".to_owned(),
    }
}

/// One event from the designer manifest.
fn event_from_designer(event: &DesignerEvent) -> EventSchema {
    EventSchema {
        name: event.name.clone(),
        payload: event.payload.map(|kind| value_type(kind, &[])),
        shape: event.shape.map(|shape| shape_token(shape).to_owned()),
    }
}

/// One event from the capability schema.
fn event_from_schema(schema: &ContractEventSchema) -> EventSchema {
    EventSchema {
        name: schema.name.to_owned(),
        payload: schema.payload.map(|kind| value_type(kind, &[])),
        shape: schema.shape.map(|shape| shape_token(shape).to_owned()),
    }
}

/// The catalog's type for a declared contract kind.
fn value_type(kind: PropertyValueKind, tokens: &[String]) -> ValueType {
    match kind {
        PropertyValueKind::Bool => ValueType::Bool,
        PropertyValueKind::Int
        | PropertyValueKind::UInt
        | PropertyValueKind::Float
        | PropertyValueKind::Number => ValueType::Number,
        PropertyValueKind::String => ValueType::Text,
        PropertyValueKind::Enum => ValueType::Enum(tokens.to_vec()),
        PropertyValueKind::Color => ValueType::Color,
        PropertyValueKind::Rect => ValueType::Rect,
        PropertyValueKind::IconSpec => ValueType::Icon,
        PropertyValueKind::Records => ValueType::Records,
        PropertyValueKind::StringList => ValueType::List {
            element: "text".to_owned(),
        },
        PropertyValueKind::NumberList => ValueType::List {
            element: "number".to_owned(),
        },
    }
}

/// A fresh control's value for a property, in the spelling the manifest writes.
///
/// Scalars and text use the library's own announcement spelling; the values with no spoken form
/// (colour, rectangle, icon, list) use the manifest's carrier spelling. `Null` stays absent, which
/// is the contract's "not set" and is not the same fact as an empty string or zero.
fn render_default(value: &CapabilityValue) -> Option<String> {
    match value {
        CapabilityValue::Null => None,
        CapabilityValue::Color(color) => Some(format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            color.r, color.g, color.b, color.a
        )),
        CapabilityValue::Rect(rect) => Some(format!(
            "{},{},{},{}",
            rect.x, rect.y, rect.width, rect.height
        )),
        CapabilityValue::IconSpec(IconSpec::Builtin(name)) => {
            Some(format!("builtin:{}", name.as_str()))
        }
        CapabilityValue::IconSpec(IconSpec::Resource { id, color_mode }) => Some(format!(
            "resource:{id} ({})",
            match color_mode {
                IconColorMode::Monochrome => "monochrome",
                IconColorMode::Original => "original",
            }
        )),
        CapabilityValue::Tuple(items) => {
            let parts: Vec<String> = items.iter().filter_map(render_default).collect();
            Some(format!("[{}]", parts.join(", ")))
        }
        CapabilityValue::Records(list) => Some(format!("{} record(s)", list.items().len())),
        other => {
            let text = other.to_announcement_string();
            if text.is_empty() { None } else { Some(text) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ANY_CHILD, CATALOG_CAPTURE_VERSION, Catalog, CommandSchema, ContractError, PropertySchema,
        Support, ValueType, WidgetDescriptor,
    };
    use rust_widgets::widget::WidgetFactory;

    fn catalog() -> Catalog {
        Catalog::of(vec![
            WidgetDescriptor::new("button", "Button", "controls")
                .with_property(PropertySchema::new("label", ValueType::Text).with_default("Button"))
                .with_property(
                    PropertySchema::new(
                        "variant",
                        ValueType::Enum(vec!["primary".to_owned(), "ghost".to_owned()]),
                    )
                    .with_access(true, false)
                    .with_tokens(vec!["primary".to_owned(), "ghost".to_owned()])
                    .with_constraint("required"),
                )
                .with_command(CommandSchema {
                    name: "click".to_owned(),
                    signature: "none".to_owned(),
                    parameter: String::new(),
                    kind: None,
                    arity: None,
                    element: None,
                    effect: "mutates-control".to_owned(),
                })
                .with_child(ANY_CHILD)
                .with_alias_of("control"),
            WidgetDescriptor::new("image", "Image", "media")
                .with_property(PropertySchema::new(
                    "source",
                    ValueType::Resource {
                        kind: "image".to_owned(),
                    },
                ))
                .unsupported("images need a device profile this target does not have")
                .with_limit("animated formats are not expressible in the contract"),
        ])
    }

    fn contract(profile: &str) -> Catalog {
        // The host factory, not the parent crate's shared table: the suites are installed there.
        Catalog::from_contract_at(
            crate::app::contract::factory(),
            profile,
            "2026-10-29T00:00:00Z",
        )
        .expect("a profile the library declares")
    }

    #[test]
    fn names_come_from_the_contract() {
        let catalog = catalog();
        assert_eq!(catalog.len(), 2);
        // A name the contract gave is found; one it did not gives nothing.
        assert_eq!(catalog.get("button").expect("button").name, "Button");
        assert!(catalog.get("invented").is_none());
        // A search for a name not in the contract returns nothing, not a plausible entry.
        assert!(catalog.search("slider").is_empty());
    }

    #[test]
    fn an_unsupported_control_has_a_reason() {
        let catalog = catalog();
        assert_eq!(catalog.unsupported().len(), 1);
        let reason = catalog.reason("image").expect("a reason");
        assert!(reason.contains("device profile"));
        // A supported control has no reason.
        assert_eq!(catalog.reason("button"), None);
        assert!(
            catalog
                .get("button")
                .expect("button")
                .support
                .is_supported()
        );
    }

    #[test]
    fn the_schema_is_the_contracts_properties() {
        let catalog = catalog();
        let schema = catalog.schema("button").expect("a schema");
        assert_eq!(schema.len(), 2);
        assert_eq!(schema[0].name, "label");
        assert_eq!(schema[0].default.as_deref(), Some("Button"));
        assert_eq!(
            catalog.schema("button").expect("schema")[1].value_type,
            ValueType::Enum(vec!["primary".to_owned(), "ghost".to_owned()])
        );
    }

    #[test]
    fn limits_are_listed_not_dropped() {
        let catalog = catalog();
        let limits = &catalog.get("image").expect("image").limits;
        assert_eq!(limits.len(), 1);
        assert!(limits[0].contains("animated formats"));
    }

    #[test]
    fn search_lifts_favorites_but_keeps_the_rest() {
        let mut catalog = catalog();
        catalog.favorite("image", true);
        let hits: Vec<&str> = catalog
            .search("")
            .iter()
            .map(|descriptor| descriptor.id.as_str())
            .collect();
        // The favorite is first, but the other kind is still findable.
        assert_eq!(hits, vec!["image", "button"]);
        assert!(catalog.is_favorite("image"));
    }

    #[test]
    fn categories_group_the_kinds() {
        let catalog = catalog();
        let categories: Vec<&str> = catalog.categories().into_iter().collect();
        assert_eq!(categories, vec!["controls", "media"]);
        assert_eq!(catalog.in_category("controls").len(), 1);
    }

    #[test]
    fn every_descriptor_is_the_manifests_own_properties_events_commands_and_children() {
        // The catalog is a view of the contract, not a second table: every column must equal the
        // manifest's own answer for the same control. This is the host-side mirror of the
        // single-creation/uniform-property principle (blue22 H-33).
        let factory = crate::app::contract::factory();
        let catalog = contract("desktop");
        for descriptor in catalog.descriptors() {
            let manifest =
                rust_widgets::widget::capability::designer_manifest(factory, &descriptor.id)
                    .expect("a registered desktop control has a manifest");

            let properties: Vec<&str> = descriptor
                .properties
                .iter()
                .map(|property| property.name.as_str())
                .collect();
            let expected: Vec<&str> = manifest
                .properties
                .iter()
                .map(|property| property.name.as_str())
                .collect();
            assert_eq!(properties, expected, "{} properties", descriptor.id);

            let events: Vec<&str> = descriptor
                .events
                .iter()
                .map(|event| event.name.as_str())
                .collect();
            let expected: Vec<&str> = manifest
                .events
                .iter()
                .map(|event| event.name.as_str())
                .collect();
            assert_eq!(events, expected, "{} events", descriptor.id);

            let commands: Vec<&str> = descriptor
                .commands
                .iter()
                .map(|command| command.name.as_str())
                .collect();
            let expected: Vec<&str> = manifest
                .commands
                .iter()
                .map(|command| command.name.as_str())
                .collect();
            assert_eq!(commands, expected, "{} commands", descriptor.id);

            assert_eq!(
                descriptor.children, manifest.children,
                "{} children",
                descriptor.id
            );
            assert_eq!(
                descriptor.alias_of, manifest.alias_of,
                "{} alias_of",
                descriptor.id
            );
            for (actual, expected) in descriptor.properties.iter().zip(&manifest.properties) {
                assert_eq!(actual.readable, expected.readable, "{}", descriptor.id);
                assert_eq!(actual.writable, expected.writable, "{}", descriptor.id);
                assert_eq!(actual.tokens, expected.tokens, "{}", descriptor.id);
                assert_eq!(
                    actual.constraints.len(),
                    expected.constraints.len(),
                    "{} constraints",
                    descriptor.id
                );
            }
        }
    }

    #[test]
    fn every_catalog_kind_is_built_by_the_one_factory() {
        let factory = crate::app::contract::factory();
        let constructible: std::collections::BTreeSet<String> = factory
            .constructible_names()
            .into_iter()
            .map(|name| name.to_string())
            .collect();
        for descriptor in contract("desktop").descriptors() {
            assert!(
                constructible.contains(&descriptor.id)
                    || matches!(descriptor.support, Support::Unsupported { .. }),
                "{} is neither constructible nor reported unsupported",
                descriptor.id
            );
        }
    }

    #[test]
    fn the_audit_gate_fails_only_on_facts_that_make_the_table_lie() {
        let clean = contract("desktop");
        assert!(
            clean.audit_problems().is_empty(),
            "{:?}",
            clean.audit_problems()
        );

        let broken = Catalog::of(vec![
            WidgetDescriptor::new("panel", "Panel", "container")
                .with_child("invented")
                .with_alias_of("missing"),
        ]);
        let problems = broken.audit_problems();
        assert!(
            problems.iter().any(|problem| problem.contains("invented")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|problem| problem.contains("missing")),
            "{problems:?}"
        );
    }

    #[test]
    fn the_audit_reports_the_capabilities_the_manifest_answered() {
        let catalog = contract("desktop");
        let audit = catalog.audit().join("\n");
        assert!(audit.contains("manifests: "), "{audit}");
        assert!(audit.contains("commands: "), "{audit}");
        assert!(audit.contains("containers: "), "{audit}");
        assert!(
            audit.contains("panel: any constructible child (*)"),
            "{audit}"
        );
        assert!(
            !audit.contains("unread:"),
            "every desktop manifest in this build is readable: {audit}"
        );
    }

    #[test]
    fn the_any_child_token_is_the_librarys_own() {
        // The catalog spells the wildcard itself so its public constant is stable, but it must not
        // drift from the manifest token the contract writes.
        assert_eq!(
            ANY_CHILD,
            rust_widgets::widget::capability::ANY_CHILD_WILDCARD
        );
    }

    #[test]
    fn a_support_reason_is_queryable_from_the_type() {
        let support = Support::Unsupported {
            reason: "not here".to_owned(),
        };
        assert_eq!(support.reason(), Some("not here"));
        assert!(!support.is_supported());
        assert_eq!(Support::Supported.reason(), None);
    }

    #[test]
    fn a_known_control_carries_its_contract_properties_and_events() {
        let catalog = contract("desktop");
        // The registry is the source: a control this crate registers is present, with the
        // properties and events the library declares for it.
        let button = catalog.get("button").expect("the contract names button");
        assert!(!button.properties.is_empty());
        assert!(
            button
                .properties
                .iter()
                .any(|property| property.name == "text"),
            "button publishes `text`: {:?}",
            button.properties
        );
        assert!(
            button.events.iter().any(|event| event.name == "clicked"),
            "button publishes `clicked`: {:?}",
            button.events
        );
        // The alias list is the contract's, not a guess.
        assert!(button.aliases.iter().any(|alias| alias == "btn"));
        // Protocol 5 publishes the designer category, so the palette no longer guesses it from
        // the kind's spelling.
        assert_eq!(button.category, "basic");
        // Commands are methods the contract publishes; a descriptor without them shows a control
        // with only half its surface.
        assert!(
            button
                .commands
                .iter()
                .any(|command| command.name == "set_text"),
            "button publishes `set_text`: {:?}",
            button.commands
        );
        // A button is a leaf; a panel is a container that accepts any constructible child.
        assert!(!button.is_container());
        assert!(!button.accepts_child("label"));
        let panel = catalog.get("panel").expect("the contract names panel");
        assert!(panel.is_container());
        assert!(panel.accepts_child("label"));
        assert!(
            catalog
                .children("panel")
                .is_some_and(|children| !children.is_empty())
        );
        // Access flags and constraints come from the contract, not from this file.
        let geometry = button
            .properties
            .iter()
            .find(|property| property.name == "geometry")
            .expect("button publishes geometry");
        assert!(
            !geometry.readable && !geometry.writable,
            "geometry is not readable or writable (readable={}, writable={})",
            geometry.readable,
            geometry.writable
        );
        let progress = catalog
            .get("progress_circle")
            .expect("the contract names progress_circle");
        let value = progress
            .properties
            .iter()
            .find(|property| property.name == "value")
            .expect("progress_circle publishes value");
        assert!(
            value
                .constraints
                .iter()
                .any(|constraint| constraint.starts_with("range ")),
            "progress_circle value declares its range: {:?}",
            value.constraints
        );
        // The aggregate accessors answer the same facts as the descriptor.
        assert!(catalog.schema("button").is_some());
        assert!(
            catalog
                .commands("button")
                .is_some_and(|commands| !commands.is_empty())
        );
        assert!(
            catalog
                .events("button")
                .expect("events")
                .iter()
                .any(|event| event.name == "clicked")
        );
    }

    #[test]
    fn the_form_suite_is_in_the_catalog_with_properties_and_events() {
        let catalog = contract("desktop");

        for (id, property, event) in [
            ("form", "title", "submitted"),
            ("form_set", "current_index", "current_changed"),
        ] {
            let descriptor = catalog
                .get(id)
                .unwrap_or_else(|| panic!("the catalog contains {id}"));
            assert!(
                descriptor.properties.iter().any(|row| row.name == property),
                "{id} publishes {property}: {:?}",
                descriptor.properties
            );
            assert!(
                descriptor.events.iter().any(|row| row.name == event),
                "{id} publishes {event}: {:?}",
                descriptor.events
            );
            // The aggregate accessors answer the same facts as the descriptor.
            assert!(
                catalog.schema(id).is_some_and(|schema| !schema.is_empty()),
                "{id} has a readable property schema"
            );
            assert!(
                catalog.events(id).is_some_and(|events| !events.is_empty()),
                "{id} has readable events"
            );
            // The manifest was readable, so no "could not be read" limit was recorded for it.
            assert!(
                descriptor.limits.is_empty(),
                "{id}: {:?}",
                descriptor.limits
            );
        }
    }

    #[test]
    fn the_form_suite_aliases_resolve_to_their_canonical_controls() {
        let catalog = contract("desktop");
        let factory = crate::app::contract::factory();

        for (alias, canonical) in [
            ("form_panel", "form"),
            ("form_layout", "form"),
            ("formset", "form_set"),
            ("wizard_form", "form_set"),
            ("form_pages", "form_set"),
        ] {
            let descriptor = catalog
                .get(canonical)
                .unwrap_or_else(|| panic!("the catalog contains {canonical}"));
            assert!(
                descriptor.aliases.iter().any(|row| row == alias),
                "{canonical} publishes the alias {alias}: {:?}",
                descriptor.aliases
            );
            assert_eq!(factory.canonical_name(alias), canonical);
            assert_eq!(
                factory.capability(alias).map(|row| row.canonical_name),
                Some(canonical)
            );
        }
    }

    #[test]
    fn an_unknown_name_is_not_invented() {
        let catalog = contract("desktop");
        // Enumerated entries are the registry's; a name it does not have yields nothing.
        assert!(catalog.get("not_a_widget").is_none());
        assert!(catalog.schema("not_a_widget").is_none());
        assert!(catalog.events("not_a_widget").is_none());
        assert!(catalog.search("not_a_widget").is_empty());

        // The library's answer is `Unknown`, and it is kept distinct from `Unavailable`.
        let support = Support::query(WidgetFactory::shared(), "not_a_widget", "desktop")
            .expect("a declared profile");
        assert!(!support.is_supported());
        assert!(!support.is_verified());
        assert!(
            support
                .reason()
                .expect("a reason")
                .contains("no capability")
        );
    }

    #[test]
    fn provenance_names_the_contract_and_the_capture() {
        let catalog = contract("desktop");
        let provenance = catalog
            .provenance()
            .expect("a captured catalog has provenance");
        assert_eq!(provenance.crate_name, crate::probe::DEPENDENCY_CRATE);
        assert_eq!(provenance.revision, crate::probe::DEPENDENCY_REVISION);
        assert!(!provenance.version.is_empty());
        assert!(provenance.protocol > 0);
        assert!(!provenance.build_profile.is_empty());
        assert!(!provenance.triple.is_empty());
        assert_eq!(provenance.target_profile, "desktop");
        assert_eq!(provenance.captured_at, "2026-10-29T00:00:00Z");
        assert_eq!(provenance.capture_version, CATALOG_CAPTURE_VERSION);
        // The report states it, so a saved output can say which contract it describes.
        assert!(catalog.render()[0].contains("contract: rust_widgets"));
        assert!(catalog.render()[0].contains("capture v"));
    }

    #[test]
    fn a_hand_built_catalog_claims_no_provenance() {
        // `Catalog::of` describes exactly what it was handed and says so by recording nothing;
        // an empty `Provenance` would read as a capture that failed to name its contract.
        assert!(catalog().provenance().is_none());
        assert_eq!(
            catalog().render()[0],
            "catalog: 2 kind(s), 1 unsupported, 0 caveat(s), 0 favorite(s)"
        );
    }

    #[test]
    fn profile_availability_is_reflected() {
        let desktop = contract("desktop");
        let button = desktop.get("button").expect("button");
        assert!(button.support.is_verified());
        assert_eq!(button.support.reason(), None);

        // A desktop build asked about `mini` cannot answer conclusively: the library says
        // `CrossProfileCaveat`, and the catalog keeps that as a caveat, not a support claim.
        let mini = contract("mini");
        let button = mini.get("button").expect("button");
        assert!(button.support.is_supported());
        assert!(!button.support.is_verified());
        assert!(
            button.support.reason().expect("a caveat").contains("mini"),
            "{:?}",
            button.support
        );
        assert!(
            mini.caveats()
                .iter()
                .any(|descriptor| descriptor.id == "button")
        );
        assert!(mini.unsupported().is_empty());
    }

    #[test]
    fn an_unknown_profile_is_refused() {
        let refusal = Catalog::from_contract(WidgetFactory::shared(), "toaster")
            .expect_err("toaster is not a profile");
        assert!(matches!(refusal, ContractError::UnknownProfile(_)));
        assert!(refusal.message().contains("not a profile"));
    }
}
