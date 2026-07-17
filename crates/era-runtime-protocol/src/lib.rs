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

pub use effect::{
    AudioEffect, AudioEffectAction, EffectAcknowledgement, EffectBatch, EffectEvent, EffectKind,
    EffectOutcome, EffectOutcomeStatus, VideoEffect,
};
pub use input::{
    AdvanceTime, DeviceStateChanged, FrontendInput, InputDeviceKind, InputIntent, InputWait,
    InteractionToken, PrimitiveInput, WaitChange, WaitKind, WaitStability,
};
pub use lifecycle::{
    ClientCapabilities, ClientHello, ClientStateChanged, CommandErrorCode, CommandRejected,
    ExecutionOrigin, ExitReason, ExitRequested, FaultCode, InputModality, ProjectionObservation,
    ProjectionState, ResynchronizeRequest, RuntimeFault, RuntimeFeature, RuntimeLimits,
    RuntimePhase, RuntimeStateChanged, SequenceAcknowledgement, ServerHello, ServiceCapability,
    ShutdownReady, ShutdownRequest, SnapshotIneligibleReason, StartMode, StartRequest,
    StateExportChunk, StateExportChunkRequest, StateExportKind, StateExportReady,
    StateExportRequest, StateExportResult, StateImportAccepted, StateImportBegin, StateImportChunk,
    StateImportCommit, StateImportReady, StateTransferCancel, StateTransferDescriptor,
    VersionRejected,
};
pub use message::{RUNTIME_PROTOCOL_VERSION, RuntimeMessage, RuntimeResynchronized};
pub use presentation::{
    AudioState, CanvasReplay, CanvasReplayCommand, CellAlignment, Color, DisplayLine, DisplayRun,
    LineAlignment, MediaPlacement, PresentationDelta, PresentationOperation, PresentationSettings,
    PresentationSnapshot, ResourceReplay, SeparatorRole, Shape, SpriteFrameReplay, SpriteReplay,
    SystemTextArgument, SystemTextKey, SystemTextRef, TextStyle, TooltipSettings,
};
pub use project::{
    DiagnosticSeverity, FileCategory, FileChange, FilePayload, FrontendIoError,
    FrontendIoErrorKind, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic, ReloadProject,
    SourceLocation, SubmittedFile, validate_relative_path,
};
pub use service::{
    CancelExternalRequest, ExternalRequestKind, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse,
    IMAGE_METADATA_OPERATION, IMAGE_METADATA_OPERATION_VERSION, IMAGE_PIXEL_OPERATION,
    IMAGE_PIXEL_OPERATION_VERSION, ImageMetadataRequest, ImageMetadataResponse, ImagePixelRequest,
    ImagePixelResponse, LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION,
    LocalDateTimeRequest, LocalDateTimeResponse, OPEN_URL_OPERATION, OPEN_URL_OPERATION_VERSION,
    OpenUrlRequest, OpenUrlResponse, POINTER_STATE_OPERATION, POINTER_STATE_OPERATION_VERSION,
    PointerStateRequest, PointerStateResponse, RANDOM_SEED_OPERATION,
    RANDOM_SEED_OPERATION_VERSION, RandomSeedRequest, RandomSeedResponse, ServiceError,
    ServiceKind, ServiceRequest, ServiceResponse, ServiceResult, StorageCapabilities, StorageEntry,
    StorageMetadata, StorageNamespace, StorageOperation, StoragePrecondition, StorageRequest,
    StorageResponse, StorageResult, UPDATE_CHECK_OPERATION, UPDATE_CHECK_OPERATION_VERSION,
    UpdateCheckRequest, UpdateCheckResponse,
};
pub use value::ProtocolValue;
