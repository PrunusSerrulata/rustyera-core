//! Caller-pumped, transport-neutral Era game runtime.
//!
//! The runtime owns the aggregate game state but performs no filesystem, clock,
//! rendering, audio, or operating-system input work. Those operations cross the
//! versioned frontend protocol and are committed only after correlated responses.

mod compiled_cache;
mod controller;
mod host;
mod input_set;
mod key_macro;
mod operation;
mod presentation;
mod project;
mod resource;
mod runtime_snapshot;
mod save_adapter;
mod session;

pub use compiled_cache::{
    DecodedProjectFile, ProjectFileError, decode_project_file,
    decode_project_file_frontend_manifest,
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
