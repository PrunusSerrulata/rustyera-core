//! Versioned message contract between the game runtime and application frontend.
//!
//! This development protocol intentionally has no backward-compatibility promise until
//! a frontend exists. Filesystem, clock, rendering and device work remain outside it.

mod effect;
mod extension;
mod input;
mod key_macro;
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
pub use erabasic_html::{
    HtmlAttribute, HtmlDocument, HtmlElementKind, HtmlInteraction, HtmlNode, parse_document,
};
pub use extension::{
    ExtensionArgument, ExtensionArgumentStyle, ExtensionCallableKind, ExtensionDeclaration,
    ExtensionInvocation, ExtensionRegistrySubmit, ExtensionResult, ExtensionValueType,
    ExtensionWrite,
};
pub use input::{
    AdvanceTime, DeviceStateChanged, FrontendInput, InputDeviceKind, InputIntent, InputUndoRequest,
    InputUndoState, InputWait, InteractionToken, PrimitiveInput, WaitChange, WaitKind,
    WaitStability,
};
pub use key_macro::{
    KEY_MACRO_GROUPS, KEY_MACRO_SLOTS, KeyMacroCommand, KeyMacroProfileSubmit, KeyMacroState,
};
pub use lifecycle::{
    ClientCapabilities, ClientHello, ClientStateChanged, CommandErrorCode, CommandRejected,
    ExecutionOrigin, ExitReason, ExitRequested, FaultCode, InputModality, ProjectionLength,
    ProjectionObservation, ProjectionSize, ProjectionState, ProjectionTransform,
    ResynchronizeRequest, RuntimeFault, RuntimeFeature, RuntimeLimits, RuntimePhase,
    RuntimeStateChanged, SequenceAcknowledgement, ServerHello, ServiceCapability, ShutdownReady,
    ShutdownRequest, SnapshotIneligibleReason, StartMode, StartRequest, StateExportChunk,
    StateExportChunkRequest, StateExportKind, StateExportReady, StateExportRequest,
    StateExportResult, StateImportAccepted, StateImportBegin, StateImportChunk, StateImportCommit,
    StateImportReady, StateTransferCancel, StateTransferDescriptor, VersionRejected,
};
pub use message::{RUNTIME_PROTOCOL_VERSION, RuntimeMessage, RuntimeResynchronized};
pub use presentation::{
    AudioState, CanvasPoint, CanvasRect, CanvasReplay, CanvasReplayCommand, CanvasSize,
    CellAlignment, Color, DisplayLine, DisplayRun, LineAlignment, LogicalLength, LogicalRect,
    MediaPlacement, PresentationDelta, PresentationLength, PresentationOperation,
    PresentationSettings, PresentationSnapshot, ResourceReplay, SeparatorRole, Shape,
    SpriteFrameReplay, SpriteReplay, SystemTextArgument, SystemTextKey, SystemTextRef, TextStyle,
    TooltipSettings,
};
pub use project::{
    DiagnosticSeverity, FileCategory, FileChange, FilePayload, FrontendIoError,
    FrontendIoErrorKind, ProjectAnalysisReport, ProjectAnalysisRequest, ProjectLoadReport,
    ProjectManifest, ProtocolDiagnostic, ReloadProject, SourceLocation, SubmittedFile,
    validate_relative_path,
};
pub use service::{
    CancelExternalRequest, CanvasPixelRequest, CanvasPixelResponse, ExternalRequestKind,
    GET_DISPLAY_LINE_OPERATION, GET_DISPLAY_LINE_OPERATION_VERSION, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GGET_TEXT_SIZE_OPERATION, GGET_TEXT_SIZE_OPERATION_VERSION,
    GetKeyStateRequest, GetKeyStateResponse, HTML_GET_PRINTED_STR_OPERATION,
    HTML_GET_PRINTED_STR_OPERATION_VERSION, HTML_STRING_LEN_OPERATION,
    HTML_STRING_LEN_OPERATION_VERSION, HTML_STRING_LINES_OPERATION,
    HTML_STRING_LINES_OPERATION_VERSION, HTML_SUBSTRING_OPERATION,
    HTML_SUBSTRING_OPERATION_VERSION, HtmlMeasureRequest, HtmlSubstringResponse,
    IMAGE_METADATA_OPERATION, IMAGE_METADATA_OPERATION_VERSION, IMAGE_PIXEL_OPERATION,
    IMAGE_PIXEL_OPERATION_VERSION, ImageMetadataRequest, ImageMetadataResponse, ImagePixelRequest,
    ImagePixelResponse, LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION,
    LocalDateTimeRequest, LocalDateTimeResponse, OPEN_URL_OPERATION, OPEN_URL_OPERATION_VERSION,
    OpenUrlRequest, OpenUrlResponse, POINTER_STATE_OPERATION, POINTER_STATE_OPERATION_VERSION,
    PointerStateRequest, PointerStateResponse, ProjectionIntegerResponse, ProjectionQueryContext,
    ProjectionStringIndexRequest, ProjectionStringResponse, RANDOM_SEED_OPERATION,
    RANDOM_SEED_OPERATION_VERSION, RandomSeedRequest, RandomSeedResponse,
    SAMPLE_CANVAS_PIXEL_OPERATION, SAMPLE_CANVAS_PIXEL_OPERATION_VERSION, ServiceError,
    ServiceKind, ServiceRequest, ServiceResponse, ServiceResult, StorageCapabilities, StorageEntry,
    StorageMetadata, StorageNamespace, StorageOperation, StoragePrecondition, StorageRequest,
    StorageResponse, StorageResult, TextExtentRequest, TextExtentResponse, UPDATE_CHECK_OPERATION,
    UPDATE_CHECK_OPERATION_VERSION, UpdateCheckRequest, UpdateCheckResponse,
};
pub use value::ProtocolValue;
