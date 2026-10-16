// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The application: what rustage does when it runs.
//!
//! This module is a boundary only. [`run`] executes a parsed command, and [`toolbar`] holds the
//! icon strip the host consumes until the designer itself exists.

pub mod adapters;
pub mod analysis_config;
pub mod arrange;
pub mod audit;
pub mod batch_replace;
pub mod binding;
pub mod chrome;
pub mod clipboard;
pub mod command;
pub mod constraints;
pub mod coordinates;
pub mod css_view;
pub mod data_model;
pub mod design;
pub mod design_tokens;
pub mod diagnostics;
pub mod diff;
pub mod dispatch;
pub mod drag;
pub mod editing;
pub mod editor_view;
pub mod events;
pub mod features;
pub mod focus;
pub mod focus_order;
pub mod frames;
pub mod groups;
pub mod highlight;
pub mod hints;
pub mod history_nav;
pub mod hot_update;
pub mod inspection;
pub mod interaction_states;
pub mod language;
pub mod layout;
pub mod layout_choice;
pub mod layout_plan;
pub mod layout_probe;
pub mod load;
pub mod menu;
pub mod motion;
pub mod multi_property;
pub mod navigation;
pub mod objects;
pub mod operations;
pub mod palette;
pub mod panels;
pub mod preview_recovery;
pub mod preview_service;
pub mod property_editor;
pub mod protocol;
pub mod quick_open;
pub mod regression;
pub mod replace_candidates;
pub mod routes;
mod run;
pub mod scale;
pub mod scene;
pub mod schema_docs;
pub mod search;
pub mod search_regression;
pub mod selection;
pub mod service_health;
pub mod session_log;
pub mod shortcuts;
pub mod snippets;
pub mod state_keep;
pub mod state_machine;
pub mod structural;
pub mod structure;
pub mod style_boundary;
pub mod style_emit;
pub mod tabs;
pub mod toolbar;
pub mod variants;
pub mod viewport;
pub mod viewport_matrix;
pub mod visual_baseline;
pub mod widget_catalog;
pub mod windows;
pub mod workbench;
pub mod workspace_edit;

pub use run::run;
