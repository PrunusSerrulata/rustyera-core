use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::{self, Write as _};
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::AtomicBool;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::Ordering;
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use era_debug_protocol::{DebugMessage, DebugResponse, DebugScope, GrantToken, ScriptOutputChunk};
use era_protocol::{
    Channel, ProtocolBytes, ProtocolError, ProtocolVersion, SessionEpoch, SessionId, VersionRange,
    WireLimits, decode_canonical, decode_envelope, encode_canonical, encode_envelope,
    negotiate_version,
};
use era_runtime_protocol::{
    AUDIO_OBSERVATION_OPERATION, AUDIO_OBSERVATION_OPERATION_VERSION, AdvanceTime, AudioChannelV1,
    AudioEffect, AudioObservationRequestV1, AudioObservationResponseV1, AudioPlaybackStateV1,
    CancelExternalRequest, CanvasPixelRequest, CanvasPixelResponse, CanvasPoint, CellAlignment,
    ClientCapabilities, ClientHello, CommandErrorCode, CommandRejected, ConfigurationApplication,
    ConfigurationClientProfile, ConfigurationUpdateCommitted, ConfigurationUpdateOutcome,
    ConfigurationUpdatePrepared, DECODE_CANVAS_IMAGE_OPERATION,
    DECODE_CANVAS_IMAGE_OPERATION_VERSION, DEVICE_PUMP_OPERATION, DEVICE_PUMP_OPERATION_VERSION,
    DecodeCanvasImageRequest, DecodeCanvasImageResponse, DevicePumpRequest, DevicePumpResponse,
    DiagnosticNotification, ENCODE_CANVAS_PNG_OPERATION, ENCODE_CANVAS_PNG_OPERATION_VERSION,
    EffectAcknowledgement, EffectBatch, EffectEvent, EffectKind, EffectOutcomeStatus,
    EncodeCanvasPngRequest, EncodeCanvasPngResponse, ExitReason, ExitRequested,
    ExtensionDeclaration, ExtensionRegistrySubmit, ExternalRequestKind, FaultCode, FileCategory,
    FilePayload, FinalizeConfigurationUpdate, FrontendInput, FrontendIoErrorKind,
    FullProjectManifest, GET_DISPLAY_LINE_OPERATION, GET_DISPLAY_LINE_OPERATION_VERSION,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GET_LINE_GEOMETRY_OPERATION,
    GET_LINE_GEOMETRY_OPERATION_VERSION, GGET_TEXT_SIZE_OPERATION,
    GGET_TEXT_SIZE_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse,
    GetLineGeometryV1Request, GetLineGeometryV1Response, HTML_GET_PRINTED_STR_OPERATION,
    HTML_GET_PRINTED_STR_OPERATION_VERSION, HTML_STRING_LEN_OPERATION,
    HTML_STRING_LEN_OPERATION_VERSION, HTML_STRING_LINES_OPERATION,
    HTML_STRING_LINES_OPERATION_VERSION, HTML_SUBSTRING_OPERATION,
    HTML_SUBSTRING_OPERATION_VERSION, IMAGE_METADATA_OPERATION, IMAGE_METADATA_OPERATION_VERSION,
    IMAGE_PIXEL_OPERATION, IMAGE_PIXEL_OPERATION_VERSION, INPUT_DEVICE_LATCH_CAPABILITY,
    INPUT_DEVICE_PUMP_CAPABILITY, INPUT_TIMED_VIEWPORT_CAPABILITY, ImageMetadataRequest,
    ImageMetadataResponse, ImagePixelRequest, ImagePixelResponse, InputIntent, InputUndoRequest,
    InputUndoState, InputWait, InteractionToken, KeyMacroCommand, KeyMacroProfileSubmit,
    LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION, LineAlignment,
    LocalDateTimeRequest, LocalDateTimeResponse, OPEN_URL_OPERATION, OPEN_URL_OPERATION_VERSION,
    OpenUrlRequest, OpenUrlResponse, POINTER_STATE_OPERATION, POINTER_STATE_OPERATION_VERSION,
    PointerStateRequest, PointerStateResponse, PrepareConfigurationUpdate, PresentationLength,
    ProjectAnalysisRequest, ProjectLoadReport, ProjectLoadRequest, ProjectManifest,
    ProjectionObservation, ProjectionQueryContext, ProjectionState, ProtocolDiagnostic,
    RANDOM_SEED_OPERATION, RANDOM_SEED_OPERATION_VERSION, RUNTIME_PROTOCOL_VERSION,
    RandomSeedRequest, RandomSeedResponse, ReloadProject, RuntimeFault, RuntimeFeature,
    RuntimeLimits, RuntimeLog, RuntimeLogLevel, RuntimeMessage, RuntimePhase,
    RuntimeResynchronized, RuntimeStateChanged, SAMPLE_CANVAS_PIXEL_OPERATION,
    SAMPLE_CANVAS_PIXEL_OPERATION_VERSION, SERIALIZE_PHYSICAL_HISTORY_OPERATION,
    SERIALIZE_PHYSICAL_HISTORY_OPERATION_VERSION, SQL_OPERATION, SQL_OPERATION_VERSION,
    SerializePhysicalHistoryRequest, SerializePhysicalHistoryResponse, ServerHello,
    ServiceCapability, ServiceKind, ServiceRequest, ServiceResponse, ServiceResult, ShutdownReady,
    SnapshotExportPurpose, SnapshotIneligibleReason, StartMode, StartRequest, StateExportCancel,
    StateExportChunk, StateExportChunkRequest, StateExportKind, StateExportReady,
    StateExportRequest, StateExportResult, StateImportAccepted, StateImportBegin, StateImportChunk,
    StateImportCommit, StateImportReady, StateTransferCancel, StateTransferDescriptor,
    StorageCapabilities, StorageEntry, StorageNamespace, StorageOperation, StoragePrecondition,
    StorageRequest, StorageResponse, StorageResult, SystemTextArgument, SystemTextKey,
    TextBoxLayout, TextExtentRequest, TextExtentResponse, UPDATE_CHECK_OPERATION,
    UPDATE_CHECK_OPERATION_VERSION, UpdateCheckRequest, UpdateCheckResponse, VersionRejected,
    WaitChange, WaitKind, WaitStability,
};
use era_runtime_protocol::{ClientPreferenceLayers, ClientPreferencesApplied, ConfigurationChange};
use erabasic_compiler::IncrementalState;
use erabasic_validator::ValidatedArtifact;
use erabasic_vm::{
    CharacterWidthMode, DEFAULT_LINE_COLUMNS, EraSaveScope, EraState, HostCallRequest,
    HostCallResult, HostReady, HostWaitStability, HostWrite, ImmediateHostCall,
    ImmediateHostCallResult, PlaceDescriptor, PreparedCandidateState, PreparedRuntimeState,
    RetainedProgramIndex, RunBudget, RuntimeVm, SnapshotEligibility, StructuredScope, VmConfig,
    VmDiagnosticNotification, VmDriveMode, VmHost, VmHostCompletion, VmHostRequest,
    VmPortDriveReport, VmPortEvent, VmPortStop, VmRestorePort, VmRuntimeFill, VmRuntimePort,
    VmRuntimeStatePort, VmRuntimeStateTransaction, VmRuntimeWrite, VmSnapshot, VmValue,
};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use era_runtime_protocol::{CONFIG_BROWSER, CONFIG_TUI};

use crate::audio::{
    AudioControl, AudioObservationContinuation, AudioObservationPurpose, AudioRuntimeState,
    control_effect, play_effect, stop_effect, volume_effect,
};
use crate::controller::{SystemController, SystemFlow, SystemStep};
use crate::host::{
    ClockOperation, ExternalCompletion, PendingInput, PointerCoordinate, PostInputAction,
    input_wait,
};
use crate::input_replay::{
    InputReplayHistory, NewGameTrigger, ReplayOrigin, ReplayOriginDetails, ReplayProject,
};
use crate::input_set::preprocess_input;
use crate::input_source::{
    InputController, InputRoot, InputSource, PendingSequence, QueuedInput, RecordedInput,
    SequenceSite,
};
use crate::key_macro::KeyMacros;
use crate::operation::{
    CandidateSaveContinuation, PendingOperations, PendingService, PendingStorage,
    SqlServiceContinuation,
};
use crate::presentation::{PresentationModel, PresentationUpdate, display_value};
use crate::project::{
    NormalizedProjectSnapshot, ProjectBuild, apply_project_delta,
    build_owned_project_with_extensions_and_progress,
};
#[cfg(test)]
use crate::project::{build_project, build_project_with_extensions_and_progress};
use crate::runtime_snapshot::{
    self, CULTURE_TABLE_VERSION, RUNTIME_SNAPSHOT_FORMAT_VERSION, RuntimeSnapshotOrigin,
    RuntimeSnapshotPayload,
};
use crate::save_adapter::{
    DecodedEraSave, decode_era_save, decode_scoped_save, encode_era_save, encode_scoped_save,
    merge_opaque_extensions, merge_structured_extensions,
};
use crate::sql::SqlRuntimeState;

fn configured_character_width_mode(
    project: Option<&NormalizedProjectSnapshot>,
) -> CharacterWidthMode {
    match project.and_then(|project| project.configuration.get_code("CharacterWidthMode")) {
        Some(era_config::ConfigValue::Enum { value, .. }) => {
            CharacterWidthMode::from_config_code(value)
        }
        _ => CharacterWidthMode::Automatic,
    }
}

mod core;
mod debug_session;
mod host_dispatch;
pub(crate) mod html_query;
mod interaction;
mod storage;
mod support;
mod transfer;
mod undo;

// Session implementation modules intentionally share this private helper facade.
#[allow(clippy::wildcard_imports)]
use support::*;

#[cfg(test)]
mod tests;

include!("session/public_api.rs");
include!("session/state.rs");
