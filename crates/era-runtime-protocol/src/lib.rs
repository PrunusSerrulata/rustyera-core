//! Versioned message contract between a future game runtime and application frontend.
//!
//! The runtime remains unimplemented. These types define the transport-neutral
//! lifecycle and keep all filesystem, clock, rendering and device work outside it.

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
    ClientHello, FaultCode, RuntimeFault, RuntimeFeature, RuntimeLimits, RuntimePhase,
    RuntimeStateChanged, ServerHello, ShutdownReady, ShutdownRequest, StartMode, StartRequest,
    StateExportKind, StateExportReady, StateExportRequest, StateExportResult, VersionRejected,
};
pub use message::{RUNTIME_PROTOCOL_VERSION, RuntimeMessage};
pub use presentation::{
    AudioState, Color, DisplayLine, DisplayRun, MediaPlacement, PresentationDelta,
    PresentationOperation, PresentationSnapshot, Shape, TextStyle,
};
pub use project::{
    DiagnosticSeverity, FileCategory, FileChange, FilePayload, FrontendIoError,
    FrontendIoErrorKind, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic, ReloadProject,
    SourceLocation, SubmittedFile, validate_relative_path,
};
pub use service::{
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest,
    GetKeyStateResponse, ServiceError, ServiceKind, ServiceRequest, ServiceResponse, ServiceResult,
    StorageEntry, StorageNamespace, StorageOperation, StorageRequest, StorageResponse,
    StorageResult,
};
pub use value::ProtocolValue;
