// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The component family's commands: definitions and instances, authoring, variants, custom widgets,
//! resource import and targets, packages, upgrade and the asset manifest.

#![allow(clippy::needless_return)]

use super::*;

/// How to define components, place instances and check for cycles (task T12.01).
#[derive(Debug, PartialEq, Eq)]
pub struct ComponentOptions {
    /// The operations: `define:<name>`, `version:<n>`, `param:<name>[=<default>]`, `slot:<name>`,
    /// `control:<kind>`, `nested:<definition>`, `useslot:<name>`, `commit`, `instance:<id>`,
    /// `of:<definition>`, `arg:<name>=<value>`, `place`, `expand:<id>`, `show`.
    pub ops: Vec<String>,
}

impl ComponentOptions {
    /// Options with nothing defined.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to extract, insert and edit a definition (task T12.02).
#[derive(Debug, PartialEq, Eq)]
pub struct ComponentAuthorOptions {
    /// The operations: `kind:<kind>`, `property:<name>=<value>`, `extract:<name>`, `use:<definition>`,
    /// `insert:<id>`, `override:<id>:<name>=<value>`, `edit:<name>`, `change:<name>=<value>`,
    /// `commit`, `resolve:<id>`, `show`.
    pub ops: Vec<String>,
}

impl ComponentAuthorOptions {
    /// Options with no selection.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to detach, reset and check a variant (task T12.03).
#[derive(Debug, PartialEq, Eq)]
pub struct ComponentOverrideOptions {
    /// The operations: `variant:<id>`, `of:<definition>`, `kind:<kind>`, `target:<name>` (repeatable),
    /// `override:<name>=<value>`, `base:<name>=<value>`, `detach`, `reset`, `check:<target>`, `show`.
    pub ops: Vec<String>,
}

impl ComponentOverrideOptions {
    /// Options with no variant.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to register a custom widget and ask whether it is publishable (task T12.04).
#[derive(Debug, PartialEq, Eq)]
pub struct CustomWidgetOptions {
    /// The operations: `widget:<name>`, `field:<name>=<type>`, `event:<name>=<type>`,
    /// `impl:<target>`, `register`, `bind:<name>=<value>`, `publish:<target>`, `validate`, `show`.
    pub ops: Vec<String>,
}

impl CustomWidgetOptions {
    /// Options with no widget.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to import a resource and replace or move it (task T12.05).
#[derive(Debug, PartialEq, Eq)]
pub struct ResourceImportOptions {
    /// The operations: `asset:<id>:<path>:<kind>:<capacity>:<bytes>`, `refer:<path>:<node>`,
    /// `replace:<id>:<path>:<bytes>`, `move:<id>:<path>`, `repoint:<from>:<to>`, `missing`, `show`.
    pub ops: Vec<String>,
}

impl ResourceImportOptions {
    /// Options with no asset.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to give a resource per-target variants and check them (task T12.06).
#[derive(Debug, PartialEq, Eq)]
pub struct ResourceTargetOptions {
    /// The operations: `resource:<name>:<kind>`, `variant:<target>:<encoding>:<bytes>`,
    /// `check:<target>`, `block:<target>`, `show`.
    pub ops: Vec<String>,
}

impl ResourceTargetOptions {
    /// Options with no resource.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to plan a package import and commit or cancel it (task T12.07).
#[derive(Debug, PartialEq, Eq)]
pub struct AssetPackageOptions {
    /// The operations: `package:<name>:<version>:<license>`, `depends:<package>`,
    /// `entry:<path>:<version>`, `existing:<path>`, `dependency:<name>:<dependency>`, `plan`,
    /// `commit`, `cancel`, `show`.
    pub ops: Vec<String>,
}

impl AssetPackageOptions {
    /// Options with no package.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to upgrade a definition and compare the round trip with the static artifact (task T12.08).
#[derive(Debug, PartialEq, Eq)]
pub struct ComponentUpgradeOptions {
    /// The operations: `definition:<name>:<version>`, `property:<name>=<value>`,
    /// `instance:<id>:<definition>`, `override:<id>:<name>=<value>`, `asset:<path>`,
    /// `change:<name>=<value>`, `upgrade:<name>`, `delete-asset:<path>`, `roundtrip`, `static`,
    /// `show`.
    pub ops: Vec<String>,
}

impl ComponentUpgradeOptions {
    /// Options with no project.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// How to build the asset manifest and its derived resources (task T12.09).
#[derive(Debug, PartialEq, Eq)]
pub struct AssetManifestOptions {
    /// The operations: `asset:<id>:<path>:<packaging>`,
    /// `derive:<id>:<source>:<transform>:<path>:<packaging>`, `move:<id>:<path>`, `resolve:<id>`,
    /// `missing:<id>`, `static`, `show`.
    pub ops: Vec<String>,
}

impl AssetManifestOptions {
    /// Options with no asset.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
}

/// Parses this family's flags, answering `None` when the flag is another family's.
pub(super) fn parse_component(
    text: &str,
    _argument: OsString,
    arguments: &mut std::vec::IntoIter<OsString>,
) -> Option<Result<Command, ParseError>> {
    macro_rules! ops {
        ($options:expr, $variant:path) => {{
            let mut options = $options;
            for argument in arguments.by_ref() {
                let value = argument.to_str().unwrap_or_default();
                if value.starts_with("--") {
                    return Some(Err(ParseError { argument }));
                }
                options.ops.push(value.to_owned());
            }
            return Some(Ok($variant(options)));
        }};
    }
    match text {
        "--component" => ops!(ComponentOptions::new(), Command::Component),
        "--component-author" => ops!(ComponentAuthorOptions::new(), Command::ComponentAuthor),
        "--component-override" => ops!(ComponentOverrideOptions::new(), Command::ComponentOverride),
        "--custom-widget" => ops!(CustomWidgetOptions::new(), Command::CustomWidget),
        "--resource-import" => ops!(ResourceImportOptions::new(), Command::ResourceImport),
        "--resource-target" => ops!(ResourceTargetOptions::new(), Command::ResourceTarget),
        "--asset-package" => ops!(AssetPackageOptions::new(), Command::AssetPackage),
        "--component-upgrade" => ops!(ComponentUpgradeOptions::new(), Command::ComponentUpgrade),
        "--asset-manifest" => ops!(AssetManifestOptions::new(), Command::AssetManifest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn parses_the_component_operations() {
        assert_eq!(
            parse(["--component"]),
            Ok(Command::Component(ComponentOptions::new()))
        );
        let mut expected = ComponentOptions::new();
        expected.ops = vec!["define:button".to_owned(), "instance:1".to_owned()];
        assert_eq!(
            parse(["--component", "define:button", "instance:1"]),
            Ok(Command::Component(expected)),
        );
        assert!(parse(["--component", "--oops"]).is_err());
    }

    #[test]
    fn parses_each_component_flag() {
        assert_eq!(
            parse(["--component-author"]),
            Ok(Command::ComponentAuthor(ComponentAuthorOptions::new()))
        );
        assert_eq!(
            parse(["--component-override"]),
            Ok(Command::ComponentOverride(ComponentOverrideOptions::new()))
        );
        assert_eq!(
            parse(["--custom-widget"]),
            Ok(Command::CustomWidget(CustomWidgetOptions::new()))
        );
        assert_eq!(
            parse(["--resource-import"]),
            Ok(Command::ResourceImport(ResourceImportOptions::new()))
        );
        assert_eq!(
            parse(["--resource-target"]),
            Ok(Command::ResourceTarget(ResourceTargetOptions::new()))
        );
        assert_eq!(
            parse(["--asset-package"]),
            Ok(Command::AssetPackage(AssetPackageOptions::new()))
        );
        assert_eq!(
            parse(["--component-upgrade"]),
            Ok(Command::ComponentUpgrade(ComponentUpgradeOptions::new()))
        );
        assert_eq!(
            parse(["--asset-manifest"]),
            Ok(Command::AssetManifest(AssetManifestOptions::new()))
        );
    }
}
