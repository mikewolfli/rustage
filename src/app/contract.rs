// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The host-owned widget factory: the pinned library's defaults plus the optional suites.
//!
//! [`rust_widgets::widget::WidgetFactory::shared`] is the *parent crate's* table. It is immutable
//! once built and therefore cannot be extended, so a control that lives in a package (the form
//! suite is one) is invisible to every lookup that reaches for it. The host therefore builds its
//! own factory once, installs the suites into it with
//! [`rust_widgets_suites::install`](rust_widgets_suites::install), and hands that table to the
//! catalog, the generator-facing lookups and the property panel.
//!
//! The factory is a `OnceLock`, not a `static mut`: `WidgetFactory` is immutable after
//! `install`, so building it once and borrowing it is exact rather than approximate. A failed
//! install is reported with the registration error the suite returned instead of being swallowed;
//! a collision means some name is already owned by another control, and building a factory
//! anyway would silently answer the wrong schema for it.

use std::sync::OnceLock;

use rust_widgets::widget::WidgetFactory;

/// The process-wide host factory.
static FACTORY: OnceLock<WidgetFactory> = OnceLock::new();

/// The host factory: the pinned library's defaults with every installed suite.
///
/// The first call builds the table; every later call returns the same borrow. This is the factory
/// the catalog, the design commands and the property panel must use — `WidgetFactory::shared()`
/// is the parent crate's table and does not contain suite controls such as `form`/`form_set`.
pub fn factory() -> &'static WidgetFactory {
    FACTORY.get_or_init(build)
}

/// Builds the factory once, reporting an install collision instead of dropping it.
fn build() -> WidgetFactory {
    let mut factory = WidgetFactory::new_with_defaults();
    if let Err(error) = rust_widgets_suites::install(&mut factory) {
        panic!("the rust-widgets-suites install failed: {error}");
    }
    factory
}

#[cfg(test)]
mod tests {
    use super::factory;

    #[test]
    fn the_host_factory_answers_the_suite_names_and_aliases() {
        let factory = factory();
        for (name, aliases) in [
            ("form", &["form_panel", "form_layout"][..]),
            ("form_set", &["formset", "wizard_form", "form_pages"][..]),
        ] {
            let capability = factory
                .capability(name)
                .unwrap_or_else(|| panic!("the host factory installs {name}"));
            assert_eq!(capability.canonical_name, name);
            for alias in aliases {
                assert_eq!(
                    factory.canonical_name(alias),
                    name,
                    "{alias} must resolve to {name}"
                );
                assert!(
                    capability.aliases.contains(alias),
                    "{name} publishes the alias {alias}"
                );
            }
        }
    }

    #[test]
    fn the_factory_is_built_once() {
        assert!(std::ptr::eq(factory(), factory()));
    }
}
