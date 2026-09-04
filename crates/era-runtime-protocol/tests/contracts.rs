use era_protocol::{
    Channel, Envelope, ProtocolBytes, ProtocolErrorCode, ProtocolVersion, SessionId,
    decode_canonical, encode_canonical,
};
use era_runtime_protocol as runtime_protocol;
use era_runtime_protocol::{
    AUDIO_OBSERVATION_OPERATION, AUDIO_OBSERVATION_OPERATION_VERSION, AdvanceTime, AudioChannelV1,
    AudioEffect, AudioEffectAction, AudioObservationRequestV1, AudioObservationResponseV1,
    AudioPlaybackStateV1, AudioState, CanvasPixelRequest, CanvasPoint, CanvasReplay,
    CanvasReplayCommand, CanvasSize, CellWidthIntent, ClientPreferenceLayers, Color,
    ConfigurationApplication, ConfigurationChange, ConfigurationClientProfile,
    ConfigurationUpdateCommitted, ConfigurationUpdateOutcome, ConfigurationValueKind,
    DiagnosticNotification, DisplayRun, EffectAcknowledgement, EffectBatch, EffectEvent,
    EffectKind, EffectOutcome, EffectOutcomeStatus, ExitReason, ExitRequested,
    FinalizeConfigurationUpdate, FrontendInput, FullProjectManifest, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GET_LINE_GEOMETRY_OPERATION,
    GET_LINE_GEOMETRY_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse,
    GetLineGeometryV1Request, GetLineGeometryV1Response, HtmlColorMatrix, InputIntent,
    InputUndoRequest, InputUndoState, InteractionToken, KeyMacroCommand, POINTER_STATE_OPERATION,
    POINTER_STATE_OPERATION_VERSION, PointerStateRequest, PointerStateResponse,
    PrepareConfigurationUpdate, PresentationDelta, PresentationOperation, PrimitiveInput,
    ProjectConfigurationEntry, ProjectConfigurationSnapshot, ProjectLoadRequest, ProjectManifest,
    ProjectionLength, ProjectionObservation, ProjectionQueryContext, ProjectionSize,
    ProjectionTransform, ProtocolDiagnostic, RUNTIME_PROTOCOL_VERSION, RedrawState, ResourceReplay,
    ReturnToTitleRequest, RuntimeFault, RuntimeLimits, RuntimeLog, RuntimeLogLevel, RuntimeMessage,
    RuntimeVmFault, RuntimeVmFaultCategory, RuntimeVmFaultCode, RuntimeVmFaultDetail,
    SAMPLE_CANVAS_PIXEL_OPERATION, SceneAnchorV1, SceneDeltaV1, SceneInteractionV1, SceneLayerV1,
    SceneOffsetV1, SceneOperationV1, SceneScrollPolicyV1, SceneSizeV1, SceneSourceV1, SceneStateV1,
    SeparatorRole, ServiceKind, ServiceRequest, SnapshotExportPurpose, SpriteFrameReplay,
    SpriteReplay, StateExportCancel, StateExportChunkRequest, StateExportKind, StateExportRequest,
    StateImportBegin, StateImportCommit, StorageNamespace, StorageOperation, StorageRequest,
    TextExtentRequest, TextStyle, parse_document, validate_relative_path,
};

include!("contracts/presentation_and_diagnostics.rs");
include!("contracts/lifecycle_and_storage.rs");
include!("contracts/audio_resources_and_sql.rs");
