// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command line: what the executable accepts.
//!
//! This module is a boundary only; [`command`] holds the parser and the usage text.

mod command;

pub use command::{
    AdapterOptions, AnalysisOptions, ArrangeOptions, BatchOptions, BindingOptions, CatalogOptions,
    ChromeOptions, ClipboardOptions, CloseOptions, Command, CommandOptions, ConfigurationOptions,
    ConflictOptions, ConstraintsOptions, CoordinatesOptions, CssOptions, DataOptions,
    DependenciesOptions, DesignOptions, DiagnosticsOptions, DiffOptions, DispatchOptions,
    DocumentEdit, DocumentOptions, DragOptions, DropEntryOptions, EditOpsOptions, EditOptions,
    EditorViewOptions, EventsOptions, FeaturesOptions, FileOpOptions, FocusOptions,
    FocusOrderOptions, FramesOptions, GroupsOptions, HealthOptions, HighlightOptions, HintsOptions,
    HistoryDomain, HistoryEdit, HistoryNavOptions, HistoryOptions, HotUpdateOptions, ImportOptions,
    IndexOptions, InspectOptions, KeysOptions, KitOptions, LanguageOptions, LayoutOptions,
    LayoutPlanOptions, LayoutProbeOptions, LayoutSwitchOptions, LoadOptions, MachineOptions,
    MatrixOptions, MotionOptions, MultiPropertyOptions, NavigationOptions, NewWorkspaceOptions,
    ObjectsOptions, OpenProjectOptions, OperationOptions, PaletteOptions, PreviewOptions,
    PreviewRecoveryOptions, PropertyEditorOptions, ProtocolOptions, QuickOpenOptions,
    RecoveryOptions, RegressionOptions, ReplaceOptions, RoutesOptions, SaveOptions, ScaleOptions,
    SceneOptions, SchemaOptions, SearchHealthOptions, SearchOptions, SessionLogOptions,
    SnippetOptions, StateKeepOptions, StatesOptions, StructuralOptions, StructureOptions,
    StyleBoundaryOptions, StyleOptions, TabsOptions, TextOptions, TokensOptions, ValidateOptions,
    VariantsOptions, ViewportOptions, VisualOptions, WindowsOptions, WorkbenchOptions,
    WorkspaceAction, WorkspaceEditOptions, WorkspaceOptions, WriteGuardOptions, help_text,
    log_level_of, parse, parse_display,
};

/// The process exit codes, defined once in the host and re-exported so a command-line caller and a
/// script agree on the numbers without a second table.
pub use crate::host::error::ExitCode;
