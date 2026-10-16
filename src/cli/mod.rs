// SPDX-FileCopyrightText: Copyright (c) 2026 mikewolfli
// SPDX-License-Identifier: MIT

//! The command line: what the executable accepts.
//!
//! This module is a boundary only; [`command`] holds the parser and the usage text.

mod command;

pub use command::{
    AdapterOptions, AnalysisOptions, ArrangeOptions, ArtifactOptions, BatchOptions, BindingOptions,
    BuildDiagOptions, CatalogOptions, ChromeOptions, ClipboardOptions, CloseOptions, Command,
    CommandOptions, CompileOptions, ConfigurationOptions, ConflictOptions, ConstraintsOptions,
    CoordinatesOptions, CssOptions, DagOptions, DataOptions, DependenciesOptions, DesignOptions,
    DiagnosticsOptions, DiffOptions, DispatchOptions, DocumentEdit, DocumentOptions, DragOptions,
    DropEntryOptions, E2eOptions, EditOpsOptions, EditOptions, EditorViewOptions, EventsOptions,
    ExportOptions, ExternalOptions, FeaturesOptions, FileOpOptions, FocusOptions,
    FocusOrderOptions, FramesOptions, GenMergeOptions, GenerateOptions, GoldenOptions,
    GroupsOptions, HeadlessOptions, HealthOptions, HighlightOptions, HintsOptions, HistoryDomain,
    HistoryEdit, HistoryNavOptions, HistoryOptions, HotUpdateOptions, ImportOptions, IndexOptions,
    InspectOptions, KeysOptions, KitOptions, LanguageOptions, LayoutOptions, LayoutPlanOptions,
    LayoutProbeOptions, LayoutSwitchOptions, LoadOptions, MachineOptions, MappingOptions,
    MatrixOptions, MotionOptions, MultiPropertyOptions, NavigationOptions, NewWorkspaceOptions,
    ObjectsOptions, OpenProjectOptions, OperationOptions, PaletteOptions, PositionOptions,
    PreviewOptions, PreviewRecoveryOptions, PropertyEditorOptions, ProtocolOptions,
    QuickOpenOptions, RecoveryOptions, RegressionOptions, ReplaceOptions, ReproducibleOptions,
    ReverseOptions, RoutesOptions, RunOptions, SaveOptions, ScaleOptions, SceneOptions,
    SchemaOptions, SearchHealthOptions, SearchOptions, SessionLogOptions, SnippetOptions,
    StateKeepOptions, StatesOptions, StructuralOptions, StructureOptions, StyleBoundaryOptions,
    StyleOptions, TabsOptions, TasksOptions, TerminalOptions, TextOptions, TokensOptions,
    ToolchainOptions, ValidateOptions, VariantsOptions, ViewportOptions, VisualOptions,
    WindowsOptions, WorkbenchOptions, WorkspaceAction, WorkspaceEditOptions, WorkspaceOptions,
    WriteGuardOptions, help_text, log_level_of, parse, parse_display,
};

/// The process exit codes, defined once in the host and re-exported so a command-line caller and a
/// script agree on the numbers without a second table.
pub use crate::host::error::ExitCode;
