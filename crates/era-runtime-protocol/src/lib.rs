//! Versioned message contract between the game runtime and application frontend.
//!
//! This development protocol intentionally has no backward-compatibility promise until
//! a frontend exists. Filesystem, clock, rendering and device work remain outside it.

mod input;
mod lifecycle;
mod message;
mod presentation;
mod project;
mod service;
mod value;

pub use input::{
    AdvanceTime, FrontendInput, InputValue, InputWait, PrimitiveInput, WaitChange, WaitKind,
    WaitStability,
};
pub use lifecycle::{
    ClientHello, CommandErrorCode, CommandRejected, FaultCode, RuntimeFault, RuntimeFeature,
    RuntimeLimits, RuntimePhase, RuntimeStateChanged, ServerHello, ShutdownReady, ShutdownRequest,
    StartMode, StartRequest, StateExportKind, StateExportReady, StateExportRequest,
    StateExportResult, VersionRejected,
};
pub use message::{RUNTIME_PROTOCOL_VERSION, RuntimeMessage};
pub use presentation::{
    AudioState, Color, DisplayLine, DisplayRun, LineAlignment, MediaPlacement, PresentationDelta,
    PresentationOperation, PresentationSettings, PresentationSnapshot, RunLayout, Shape, TextStyle,
};
pub use project::{
    DiagnosticSeverity, FileCategory, FileChange, FilePayload, FrontendIoError,
    FrontendIoErrorKind, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic, ReloadProject,
    SourceLocation, SubmittedFile, validate_relative_path,
};
pub use service::{
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest,
    GetKeyStateResponse, LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION,
    LocalDateTimeRequest, LocalDateTimeResponse, RANDOM_SEED_OPERATION,
    RANDOM_SEED_OPERATION_VERSION, RandomSeedRequest, RandomSeedResponse, ServiceError,
    ServiceKind, ServiceRequest, ServiceResponse, ServiceResult, StorageEntry, StorageNamespace,
    StorageOperation, StorageRequest, StorageResponse, StorageResult,
};
pub use value::ProtocolValue;
