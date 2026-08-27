//! Caller-pumped, transport-neutral Era game runtime.
//!
//! The runtime owns the aggregate game state but performs no filesystem, clock,
//! rendering, audio, or operating-system input work. Those operations cross the
//! versioned frontend protocol and are committed only after correlated responses.

/// Product version embedded in this runtime build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod compatibility;
mod compiled_cache;
mod controller;
mod host;
mod input_replay;
mod input_set;
mod key_macro;
mod operation;
mod presentation;
mod project;
mod resource;
mod runtime_snapshot;
mod save_adapter;
mod session;

pub use compatibility::{compatibility_configuration_digest, resolve_project_compatibility};
pub use compiled_cache::{
    DecodedProjectFile, DecodedProjectFileStream, ProjectConfigurationUpdate, ProjectFileError,
    ProjectFileStreamDecoder, decode_project_file, decode_project_file_frontend_manifest,
    prepare_project_configuration_update,
};
pub use runtime_snapshot::{
    RUNTIME_SNAPSHOT_INSPECTION_SCHEMA_VERSION, RuntimeSnapshotContainerInspection,
    RuntimeSnapshotInspection, RuntimeSnapshotInspectionError, RuntimeSnapshotValidation,
    inspect_runtime_snapshot,
};
pub use session::{
    ProjectProgress, ProjectProgressReporter, ProjectProgressStage, RuntimeDriveBudget,
    RuntimeDriveReport, RuntimeDriveState, RuntimeError, RuntimeOptions, RuntimeSession,
    TraditionalSaveInspection, TraditionalSaveValidationError,
};
