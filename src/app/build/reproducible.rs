// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! Reproducible and offline builds (task T16.10).
//!
//! The acceptance line is *"a locked, offline build uses the committed lock file, reaches no network,
//! and keeps its cache outside the committed source"*.
//!
//! * **The flags are derived, not typed twice.** [`Reproducibility::flags`] turns the settings into
//!   cargo's own flags: `--locked` for a lock file that must not change, `--frozen` for locked **and**
//!   offline, `--offline` for offline alone. A caller cannot ask for a combination cargo does not
//!   accept, because it never spells the flags itself.
//! * **The cache boundary is checked.** A target directory inside the project's source would be
//!   committed and would make a build depend on what a previous machine left there, so
//!   [`Reproducibility::cache_is_outside_source`] refuses it.
//! * **A vendored source is a directory, stated.** [`Reproducibility::vendor`] names it rather than
//!   setting an environment variable at a distance, so the `.cargo/config.toml` a project needs is
//!   something a reader can see.

#![allow(dead_code)]

/// A build's reproducibility settings.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reproducibility {
    /// Whether `Cargo.lock` must be present and unchanged.
    pub locked: bool,
    /// Whether the network must not be reached.
    pub offline: bool,
    /// A vendored source directory, project-relative.
    pub vendor: Option<String>,
    /// The cache (target) directory, project-relative.
    pub cache: Option<String>,
    /// The source directory whose contents are committed.
    pub source: Option<String>,
}

impl Default for Reproducibility {
    /// A build that may change the lock file and may reach the network.
    fn default() -> Self {
        Self {
            locked: false,
            offline: false,
            vendor: None,
            cache: None,
            source: None,
        }
    }
}

impl Reproducibility {
    /// The cargo flags these settings imply, without the subcommand.
    ///
    /// `frozen` implies both `locked` and `offline`, so it is emitted alone when both hold; asking
    /// for `--locked --frozen` would be two flags for one meaning.
    pub fn flags(&self) -> Vec<String> {
        if self.locked && self.offline {
            return vec!["--frozen".to_owned()];
        }
        let mut flags = Vec::new();
        if self.locked {
            flags.push("--locked".to_owned());
        }
        if self.offline {
            flags.push("--offline".to_owned());
        }
        flags
    }

    /// Whether the cache lives outside the committed source.
    ///
    /// A cache inside the source is refused: it would be committed, and a build would then depend on
    /// artifacts a different machine happened to leave behind.
    pub fn cache_is_outside_source(&self) -> bool {
        match (&self.cache, &self.source) {
            (Some(cache), Some(source)) => {
                let cache = cache.trim_matches('/');
                let source = source.trim_matches('/');
                cache != source && !cache.starts_with(&format!("{source}/"))
            }
            // With no source named, the question has no answer to refuse on; the caller decides.
            _ => true,
        }
    }

    /// The `.cargo/config.toml` a vendored build needs, as lines.
    pub fn vendor_config(&self) -> Vec<String> {
        match &self.vendor {
            Some(vendor) => vec![
                "[source.crates-io]".to_owned(),
                "replace-with = \"vendored-sources\"".to_owned(),
                String::new(),
                "[source.vendored-sources]".to_owned(),
                format!("directory = \"{vendor}\""),
            ],
            None => Vec::new(),
        }
    }

    /// The settings in one line per fact.
    pub fn render(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.push(format!(
            "flags: {}",
            if self.flags().is_empty() {
                "none (the lock file and the network are both mutable)".to_owned()
            } else {
                self.flags().join(" ")
            }
        ));
        out.push(format!(
            "cache {} the committed source",
            if self.cache_is_outside_source() {
                "is outside"
            } else {
                "is inside"
            }
        ));
        if let Some(vendor) = &self.vendor {
            out.push(format!("vendor: {vendor}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Reproducibility;

    #[test]
    fn locked_alone_is_locked_and_offline_alone_is_offline() {
        let locked = Reproducibility {
            locked: true,
            ..Default::default()
        };
        assert_eq!(locked.flags(), vec!["--locked"]);
        let offline = Reproducibility {
            offline: true,
            ..Default::default()
        };
        assert_eq!(offline.flags(), vec!["--offline"]);
    }

    #[test]
    fn locked_and_offline_is_the_one_frozen_flag() {
        let frozen = Reproducibility {
            locked: true,
            offline: true,
            ..Default::default()
        };
        assert_eq!(frozen.flags(), vec!["--frozen"]);
    }

    #[test]
    fn no_settings_means_no_flags() {
        assert!(Reproducibility::default().flags().is_empty());
    }

    #[test]
    fn a_cache_inside_the_source_is_refused_and_one_outside_is_allowed() {
        let inside = Reproducibility {
            cache: Some("src/target".to_owned()),
            source: Some("src".to_owned()),
            ..Default::default()
        };
        assert!(!inside.cache_is_outside_source());
        assert!(inside.render()[1].contains("inside"));

        let outside = Reproducibility {
            cache: Some("target".to_owned()),
            source: Some("src".to_owned()),
            ..Default::default()
        };
        assert!(outside.cache_is_outside_source());
    }

    #[test]
    fn a_vendored_build_states_its_config() {
        let vendored = Reproducibility {
            vendor: Some("vendor".to_owned()),
            ..Default::default()
        };
        let config = vendored.vendor_config();
        assert!(config.iter().any(|line| line.contains("vendored-sources")));
        assert!(
            config
                .iter()
                .any(|line| line.contains("directory = \"vendor\""))
        );
        assert!(Reproducibility::default().vendor_config().is_empty());
    }
}
