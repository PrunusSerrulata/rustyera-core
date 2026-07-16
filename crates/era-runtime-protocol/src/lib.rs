//! Versioned message contract between the game runtime and application frontend.
//!
//! This development protocol intentionally has no backward-compatibility promise until
//! a frontend exists. Filesystem, clock, rendering and device work remain outside it.

mod effect;
mod input;
mod lifecycle;
mod message;
mod presentation;
mod project;
mod service;
mod value;

pub use effect::{EffectAcknowledgement, EffectBatch, EffectEvent, EffectKind};
pub use input::{
    AdvanceTime, DeviceStateChanged, FrontendInput, InputDeviceKind, InputIntent, InputWait,
    InteractionToken, PrimitiveInput, WaitChange, WaitKind, WaitStability,
};
pub use lifecycle::{
    ClientCapabilities, ClientHello, ClientStateChanged, CommandErrorCode, CommandRejected,
    ExitReason, ExitRequested, FaultCode, InputModality, ResynchronizeRequest, RuntimeFault,
    RuntimeFeature, RuntimeLimits, RuntimePhase, RuntimeStateChanged, SequenceAcknowledgement,
    ServerHello, ShutdownReady, ShutdownRequest, SnapshotIneligibleReason, StartMode, StartRequest,
    StateExportChunk, StateExportChunkRequest, StateExportKind, StateExportReady,
    StateExportRequest, StateExportResult, StateImportAccepted, StateImportBegin, StateImportChunk,
    StateImportCommit, StateImportReady, StateTransferCancel, StateTransferDescriptor,
    VersionRejected,
};
pub use message::{RUNTIME_PROTOCOL_VERSION, RuntimeMessage, RuntimeResynchronized};
pub use presentation::{
    AudioState, CellAlignment, Color, DisplayLine, DisplayRun, LineAlignment, MediaPlacement,
    PresentationDelta, PresentationOperation, PresentationSettings, PresentationSnapshot,
    RunLayout, SeparatorRole, Shape, SystemTextArgument, SystemTextKey, SystemTextRef, TextStyle,
};
pub use project::{
    DiagnosticSeverity, FileCategory, FileChange, FilePayload, FrontendIoError,
    FrontendIoErrorKind, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic, ReloadProject,
    SourceLocation, SubmittedFile, validate_relative_path,
};
pub use service::{
    CancelExternalRequest, ExternalRequestKind, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse,
    LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION, LocalDateTimeRequest,
    LocalDateTimeResponse, RANDOM_SEED_OPERATION, RANDOM_SEED_OPERATION_VERSION, RandomSeedRequest,
    RandomSeedResponse, ServiceError, ServiceKind, ServiceRequest, ServiceResponse, ServiceResult,
    StorageEntry, StorageMetadata, StorageNamespace, StorageOperation, StoragePrecondition,
    StorageRequest, StorageResponse, StorageResult,
};
pub use value::ProtocolValue;
