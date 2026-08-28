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
    AdvanceTime, AudioEffect, AudioEffectAction, CancelExternalRequest, CanvasPixelRequest,
    CanvasPixelResponse, CanvasPoint, CellAlignment, ClientCapabilities, ClientHello,
    CommandErrorCode, CommandRejected, ConfigurationApplication, ConfigurationClientProfile,
    ConfigurationUpdateCommitted, ConfigurationUpdateOutcome, ConfigurationUpdatePrepared,
    DECODE_CANVAS_IMAGE_OPERATION, DECODE_CANVAS_IMAGE_OPERATION_VERSION, DecodeCanvasImageRequest,
    DecodeCanvasImageResponse, DiagnosticNotification, ENCODE_CANVAS_PNG_OPERATION,
    ENCODE_CANVAS_PNG_OPERATION_VERSION, EffectAcknowledgement, EffectBatch, EffectEvent,
    EffectKind, EffectOutcomeStatus, EncodeCanvasPngRequest, EncodeCanvasPngResponse, ExitReason,
    ExitRequested, ExtensionDeclaration, ExtensionRegistrySubmit, ExternalRequestKind, FaultCode,
    FileCategory, FilePayload, FinalizeConfigurationUpdate, FrontendInput, FrontendIoErrorKind,
    FullProjectManifest, GET_DISPLAY_LINE_OPERATION, GET_DISPLAY_LINE_OPERATION_VERSION,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GGET_TEXT_SIZE_OPERATION,
    GGET_TEXT_SIZE_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse,
    HTML_GET_PRINTED_STR_OPERATION, HTML_GET_PRINTED_STR_OPERATION_VERSION,
    HTML_STRING_LEN_OPERATION, HTML_STRING_LEN_OPERATION_VERSION, HTML_STRING_LINES_OPERATION,
    HTML_STRING_LINES_OPERATION_VERSION, HTML_SUBSTRING_OPERATION,
    HTML_SUBSTRING_OPERATION_VERSION, IMAGE_METADATA_OPERATION, IMAGE_METADATA_OPERATION_VERSION,
    IMAGE_PIXEL_OPERATION, IMAGE_PIXEL_OPERATION_VERSION, ImageMetadataRequest,
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
    SERIALIZE_PHYSICAL_HISTORY_OPERATION_VERSION, SerializePhysicalHistoryRequest,
    SerializePhysicalHistoryResponse, ServerHello, ServiceCapability, ServiceKind, ServiceRequest,
    ServiceResponse, ServiceResult, ShutdownReady, SnapshotExportPurpose, SnapshotIneligibleReason,
    StartMode, StartRequest, StateExportCancel, StateExportChunk, StateExportChunkRequest,
    StateExportKind, StateExportReady, StateExportRequest, StateExportResult, StateImportAccepted,
    StateImportBegin, StateImportChunk, StateImportCommit, StateImportReady, StateTransferCancel,
    StateTransferDescriptor, StorageCapabilities, StorageEntry, StorageNamespace, StorageOperation,
    StoragePrecondition, StorageRequest, StorageResponse, StorageResult, SystemTextArgument,
    SystemTextKey, TextBoxLayout, TextExtentRequest, TextExtentResponse, UPDATE_CHECK_OPERATION,
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

use crate::controller::{SystemController, SystemFlow, SystemStep};
use crate::host::{
    ClockOperation, ExternalCompletion, PendingInput, PointerCoordinate, PostInputAction,
    input_wait,
};
use crate::input_replay::{
    InputReplayHistory, NewGameTrigger, ReplayOrigin, ReplayOriginDetails, ReplayProject,
};
use crate::input_set::{InputSegment, preprocess_input};
use crate::key_macro::KeyMacros;
use crate::operation::{
    CandidateSaveContinuation, PendingOperations, PendingService, PendingStorage,
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

#[derive(Clone, Copy, Debug)]
pub struct RuntimeOptions {
    pub session_id: SessionId,
    pub limits: RuntimeLimits,
    pub wire_limits: WireLimits,
    pub vm_config: VmConfig,
    /// Creator-owned upper bound for [`DebugScope`] discriminants.
    pub debug_scope_mask: u64,
    /// Keep complete project file payloads in the reload snapshot after a successful build.
    ///
    /// Constrained hosts that can rematerialize an authorized project may disable this and submit
    /// a complete payload set for each reload. Paths, hashes, resource descriptors, and revisions
    /// remain available in the compact snapshot.
    pub retain_project_source_payloads: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectProgressStage {
    Scanning,
    Normalizing,
    LoadingData,
    Parsing,
    Analyzing,
    Compiling,
    Validating,
    Finalizing,
    Preparing,
    Packaging,
    CacheParsing,
    CacheDecoding,
    CacheValidating,
    InitializingMemory,
    IndexingProgram,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProgress {
    pub stage: ProjectProgressStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Clone)]
pub struct ProjectProgressReporter {
    #[cfg(not(target_arch = "wasm32"))]
    callback: Arc<dyn Fn(ProjectProgress) + Send + Sync>,
    #[cfg(not(target_arch = "wasm32"))]
    gate: Arc<std::sync::Mutex<ProjectProgressGate>>,
    #[cfg(not(target_arch = "wasm32"))]
    started_at: Instant,
    #[cfg(target_arch = "wasm32")]
    callback: std::rc::Rc<dyn Fn(ProjectProgress)>,
    #[cfg(target_arch = "wasm32")]
    gate: std::rc::Rc<std::cell::RefCell<ProjectProgressGate>>,
    #[cfg(target_arch = "wasm32")]
    elapsed: std::rc::Rc<dyn Fn() -> Duration>,
}

#[derive(Default)]
struct ProjectProgressGate {
    last: Option<ProjectProgress>,
    last_emitted_at: Option<Duration>,
}

impl ProjectProgressGate {
    const INTERVAL: Duration = Duration::from_millis(34);

    fn accepts(&mut self, progress: ProjectProgress, now: Duration) -> bool {
        if self.last == Some(progress) {
            return false;
        }
        let boundary = progress.completed == 0 || progress.completed >= progress.total;
        let stage_changed = self
            .last
            .is_none_or(|previous| previous.stage != progress.stage);
        if !stage_changed
            && self
                .last
                .is_some_and(|previous| progress.completed < previous.completed)
        {
            return false;
        }
        let interval_elapsed = self
            .last_emitted_at
            .is_none_or(|previous| now.saturating_sub(previous) >= Self::INTERVAL);
        let accepts = stage_changed || boundary || interval_elapsed;
        if accepts {
            self.last = Some(progress);
            self.last_emitted_at = Some(now);
        }
        accepts
    }
}

impl ProjectProgressReporter {
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn new(callback: impl Fn(ProjectProgress) + Send + Sync + 'static) -> Self {
        Self {
            callback: Arc::new(callback),
            gate: Arc::new(std::sync::Mutex::new(ProjectProgressGate::default())),
            started_at: Instant::now(),
        }
    }

    /// Create a reporter with a host clock, retaining the native monotonic clock off WebAssembly.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn new_with_elapsed(
        callback: impl Fn(ProjectProgress) + Send + Sync + 'static,
        _elapsed: impl Fn() -> Duration + 'static,
    ) -> Self {
        Self::new(callback)
    }

    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn new(callback: impl Fn(ProjectProgress) + 'static) -> Self {
        Self::new_with_elapsed(callback, || Duration::ZERO)
    }

    /// Create a WebAssembly reporter with a host-provided monotonic elapsed clock.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn new_with_elapsed(
        callback: impl Fn(ProjectProgress) + 'static,
        elapsed: impl Fn() -> Duration + 'static,
    ) -> Self {
        Self {
            callback: std::rc::Rc::new(callback),
            gate: std::rc::Rc::new(std::cell::RefCell::new(ProjectProgressGate::default())),
            elapsed: std::rc::Rc::new(elapsed),
        }
    }

    pub(crate) fn report(&self, progress: ProjectProgress) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut gate = self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if gate.accepts(progress, self.started_at.elapsed()) {
                // Serialize the callback with acceptance so concurrent producers cannot
                // reorder accepted updates while crossing an FFI or IPC boundary.
                (self.callback)(progress);
            }
        }
        #[cfg(target_arch = "wasm32")]
        if self.gate.borrow_mut().accepts(progress, (self.elapsed)()) {
            (self.callback)(progress);
        }
    }
}

#[cfg(test)]
mod progress_reporter_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, Mutex};

    use super::*;

    #[test]
    fn project_progress_coalesces_duplicates_and_keeps_stage_boundaries() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&reports);
        let reporter = ProjectProgressReporter::new(move |progress| {
            observed.lock().unwrap().push(progress);
        });
        for completed in 0..=1_000 {
            let progress = ProjectProgress {
                stage: ProjectProgressStage::Compiling,
                completed,
                total: 1_000,
            };
            reporter.report(progress);
            reporter.report(progress);
        }
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Finalizing,
            completed: 0,
            total: 10,
        });
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Finalizing,
            completed: 10,
            total: 10,
        });

        let reports = reports.lock().unwrap();
        assert_eq!(reports.first().unwrap().completed, 0);
        assert_eq!(
            reports[reports.len() - 2].stage,
            ProjectProgressStage::Finalizing
        );
        assert_eq!(reports[reports.len() - 2].completed, 0);
        assert_eq!(reports.last().unwrap().completed, 10);
        assert!(reports.len() <= 4);
        assert!(reports.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn project_progress_gate_uses_time_and_preserves_boundaries() {
        let mut gate = ProjectProgressGate::default();
        let compiling = |completed, total| ProjectProgress {
            stage: ProjectProgressStage::Compiling,
            completed,
            total,
        };

        assert!(gate.accepts(compiling(0, 100), Duration::ZERO));
        assert!(!gate.accepts(compiling(1, 100), Duration::from_millis(33)));
        assert!(gate.accepts(compiling(2, 100), Duration::from_millis(34)));
        assert!(!gate.accepts(compiling(1, 100), Duration::from_secs(1)));
        assert!(gate.accepts(compiling(100, 100), Duration::from_millis(35)));
        assert!(!gate.accepts(compiling(100, 100), Duration::from_secs(2)));

        let zero_total = ProjectProgress {
            stage: ProjectProgressStage::Preparing,
            completed: 0,
            total: 0,
        };
        assert!(gate.accepts(zero_total, Duration::from_millis(35)));
        assert!(!gate.accepts(zero_total, Duration::from_secs(3)));
    }

    #[test]
    fn project_progress_callback_is_serialized_across_threads() {
        const THREADS: usize = 8;
        let active = Arc::new(AtomicUsize::new(0));
        let maximum_active = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(Mutex::new(Vec::new()));
        let callback_active = Arc::clone(&active);
        let callback_maximum = Arc::clone(&maximum_active);
        let callback_observed = Arc::clone(&observed);
        let reporter = Arc::new(ProjectProgressReporter::new(move |progress| {
            let current = callback_active.fetch_add(1, Ordering::SeqCst) + 1;
            callback_maximum.fetch_max(current, Ordering::SeqCst);
            std::thread::yield_now();
            callback_observed.lock().unwrap().push(progress);
            callback_active.fetch_sub(1, Ordering::SeqCst);
        }));
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles = (0..THREADS)
            .map(|index| {
                let reporter = Arc::clone(&reporter);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    reporter.report(ProjectProgress {
                        stage: if index % 2 == 0 {
                            ProjectProgressStage::Parsing
                        } else {
                            ProjectProgressStage::Analyzing
                        },
                        completed: 0,
                        total: 1,
                    });
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(maximum_active.load(Ordering::SeqCst), 1);
        assert!(!observed.lock().unwrap().is_empty());
    }

    #[test]
    fn project_progress_reporter_allows_an_intermediate_after_the_interval() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&reports);
        let reporter = ProjectProgressReporter::new(move |progress| {
            observed.lock().unwrap().push(progress);
        });
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Compiling,
            completed: 0,
            total: 100,
        });
        std::thread::sleep(ProjectProgressGate::INTERVAL);
        reporter.report(ProjectProgress {
            stage: ProjectProgressStage::Compiling,
            completed: 1,
            total: 100,
        });
        assert_eq!(reports.lock().unwrap().len(), 2);
    }
}

#[cfg(test)]
mod tests;

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            session_id: SessionId { high: 0, low: 1 },
            limits: RuntimeLimits {
                maximum_envelope_bytes: 1024 * 1024 * 1024,
                maximum_payload_bytes: 1023 * 1024 * 1024,
                maximum_pending_requests: 1024,
                maximum_journal_entries: 4096,
                maximum_drive_instructions: 100_000,
                maximum_transfer_bytes: 1024 * 1024 * 1024,
                maximum_journal_bytes: 64 * 1024 * 1024,
            },
            wire_limits: WireLimits::default(),
            vm_config: VmConfig::default(),
            debug_scope_mask: 0,
            retain_project_source_payloads: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeDriveBudget {
    pub maximum_vm_instructions: u64,
    pub maximum_runtime_transitions: u32,
}

impl Default for RuntimeDriveBudget {
    fn default() -> Self {
        Self {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeDriveState {
    Idle,
    MoreWork,
    OutputReady,
    Stopped,
    Faulted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraditionalSaveInspection {
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraditionalSaveValidationError {
    ProjectUnavailable,
    Invalid(String),
    DifferentGame,
    DifferentVersion,
    Incompatible(String),
}

impl fmt::Display for TraditionalSaveValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectUnavailable => formatter.write_str("no compiled project is available"),
            Self::Invalid(message) => write!(formatter, "traditional save is invalid: {message}"),
            Self::DifferentGame => {
                formatter.write_str("traditional save belongs to a different game")
            }
            Self::DifferentVersion => {
                formatter.write_str("traditional save belongs to an incompatible game version")
            }
            Self::Incompatible(message) => {
                write!(formatter, "traditional save is incompatible: {message}")
            }
        }
    }
}

impl std::error::Error for TraditionalSaveValidationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeDriveReport {
    pub state: RuntimeDriveState,
    pub vm_instructions: u64,
    pub runtime_transitions: u32,
    pub queued_envelopes: u32,
    /// Whether this drive advanced one single-threaded background-work quantum.
    pub cooperative_background_work: bool,
}

#[derive(Debug)]
pub enum RuntimeError {
    Protocol(ProtocolError),
    InvalidSequence {
        expected: u64,
        actual: u64,
    },
    SessionMismatch,
    ResourceLimit(&'static str),
    Busy(&'static str),
    /// A trusted script domain/read failure; only the Host dispatch boundary catches it.
    Script {
        kind: erabasic_vm::ScriptFaultKind,
        message: String,
    },
    Internal(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => error.fmt(formatter),
            Self::InvalidSequence { expected, actual } => {
                write!(formatter, "expected sequence {expected}, received {actual}")
            }
            Self::SessionMismatch => formatter.write_str("runtime session identity differs"),
            Self::ResourceLimit(message) | Self::Busy(message) => formatter.write_str(message),
            Self::Script { message, .. } | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<ProtocolError> for RuntimeError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionState {
    Negotiating,
    Active,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
enum SystemMenuState {
    #[default]
    Title,
    LoadSlots,
    SaveSlots,
    ConfirmOverwrite {
        slot: u32,
    },
}

#[derive(Debug)]
struct InboundStateTransfer {
    descriptor: StateTransferDescriptor,
    bytes: Vec<u8>,
    hasher: Option<blake3::Hasher>,
    committed: bool,
}

#[derive(Debug)]
struct OutboundStateTransfer {
    descriptor: StateTransferDescriptor,
    bytes: Arc<Vec<u8>>,
    next_offset: u64,
}

#[derive(Debug)]
struct StagedFullProjectManifest {
    source_transfer_id: Option<u64>,
    manifest: ProjectManifest,
}

enum ProjectContainerTask {
    #[cfg(not(target_arch = "wasm32"))]
    Native {
        cancelled: Arc<AtomicBool>,
        handle: Option<JoinHandle<Result<Vec<u8>, String>>>,
    },
    #[cfg(any(target_arch = "wasm32", test))]
    Cooperative {
        encoder: Box<crate::compiled_cache::CooperativeCompiledCacheEncoder>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ProjectContainerTask {
    fn drop(&mut self) {
        match self {
            Self::Native {
                cancelled,
                handle: _,
            } => {
                cancelled.store(true, Ordering::Relaxed);
            }
            #[cfg(test)]
            Self::Cooperative { .. } => {}
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum InboundMessage {
    Runtime(RuntimeMessage),
    Debug(DebugMessage),
}

#[derive(Clone, Debug)]
struct ActiveDebugGrant {
    token: GrantToken,
    scopes: BTreeSet<DebugScope>,
}

/// Single-owner runtime actor. Methods only enqueue, drive, and dequeue messages;
/// no frontend code can run inside a VM instruction dispatch.
#[allow(clippy::struct_excessive_bools)]
pub struct RuntimeSession {
    options: RuntimeOptions,
    project_progress_reporter: Option<ProjectProgressReporter>,
    state: SessionState,
    phase: RuntimePhase,
    revision: u64,
    epoch: SessionEpoch,
    expected_inbound_sequence: u64,
    expected_debug_sequence: u64,
    outbound_sequence: u64,
    debug_outbound_sequence: u64,
    next_message_id: u64,
    next_request_id: u64,
    next_wait_id: u64,
    next_interaction_id: u64,
    next_transfer_id: u64,
    next_effect_id: u64,
    logical_time_ns: u64,
    frontend_time_origin: Option<(u64, u64)>,
    random_seed: Option<u64>,
    input_replay: InputReplayHistory,
    next_new_game_trigger: NewGameTrigger,
    negotiated_features: BTreeSet<RuntimeFeature>,
    configuration_profile: ConfigurationClientProfile,
    client_preferences: Option<ClientPreferenceLayers>,
    inbound: VecDeque<(u64, InboundMessage)>,
    outbound: VecDeque<Vec<u8>>,
    outbound_journal: BTreeMap<u64, Vec<u8>>,
    outbound_journal_bytes: u64,
    effect_journal: BTreeMap<u64, EffectEvent>,
    accepted_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    accepted_debug_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    active_debug_grant: Option<ActiveDebugGrant>,
    next_debug_grant_id: u64,
    debug_resume_phase: Option<RuntimePhase>,
    debug_frontend_time_sample: Option<u64>,
    artifact: Option<ValidatedArtifact>,
    incremental: Arc<IncrementalState>,
    extension_declarations: Vec<ExtensionDeclaration>,
    vm: Option<RuntimeVm>,
    retained_title_program: Option<RetainedProgramIndex>,
    presentation: PresentationModel,
    pending_presentation_update: bool,
    operations: PendingOperations,
    key_toggle_state: [u8; 256],
    hotkey_state: Vec<i64>,
    key_macros: KeyMacros,
    queued_input: VecDeque<InputSegment>,
    deferred_input_completion: Option<InputSubmission>,
    text_box: String,
    text_box_layout: TextBoxLayout,
    flow_input_enabled: bool,
    flow_input_default: i64,
    flow_input_can_skip: bool,
    flow_input_force_skip: bool,
    flow_input_string: bool,
    flow_input_default_string: String,
    button_generation: u64,
    debug_output: String,
    debug_output_base: u64,
    debug_output_subscribed: bool,
    projection_environment_revision: u64,
    projection_space_revision: u64,
    last_projection_state: Option<ProjectionState>,
    client_width: u32,
    client_height: u32,
    line_columns: u32,
    message_skip: bool,
    skip_print: bool,
    user_defined_skip: bool,
    saved_skip: bool,
    force_kana_mode: u8,
    client_focused: bool,
    client_audio_available: bool,
    command_intents: BTreeMap<InteractionToken, VmValue>,
    reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    exit_requested: Option<ExitRequested>,
    controller: SystemController,
    undo_checkpoint: Option<UndoCheckpoint>,
    undo_replay: Option<UndoReplay>,
    undo_token: Option<InteractionToken>,
    project_snapshot: Option<NormalizedProjectSnapshot>,
    pending_configuration_update: Option<PendingConfigurationUpdate>,
    selected_locale: String,
    available_fonts: BTreeSet<String>,
    service_capabilities: BTreeMap<(ServiceKind, String), ProtocolVersion>,
    storage_capabilities: StorageCapabilities,
    save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    system_menu: SystemMenuState,
    load_slot_paths: Vec<String>,
    occupied_slot_paths: BTreeSet<String>,
    slot_change_tokens: BTreeMap<String, String>,
    slot_labels: BTreeMap<String, String>,
    invalid_slot_paths: BTreeSet<String>,
    system_menu_host_request: Option<erabasic_vm::HostRequestId>,
    system_menu_page: u32,
    inbound_transfer: Option<InboundStateTransfer>,
    outbound_transfer: Option<OutboundStateTransfer>,
    staged_project_manifest: Option<ProjectManifest>,
    staged_project_file_cache: Option<crate::compiled_cache::DecodedCompiledCache>,
    staged_full_project_manifest: Option<StagedFullProjectManifest>,
    pending_project_load: Option<PendingProjectLoad>,
    pending_candidate_commit: Option<PendingCandidateCommit>,
    candidate_clock: Option<LocalDateTimeResponse>,
    compiled_project_cache: Option<Arc<Vec<u8>>>,
    compiled_cache_diagnostics: Vec<ProtocolDiagnostic>,
    compiled_cache_task: Option<ProjectContainerTask>,
    compiled_cache_failure: Option<String>,
    full_project_file: Option<Arc<Vec<u8>>>,
    full_project_task: Option<ProjectContainerTask>,
    full_project_failure: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct UndoCheckpoint {
    slot: u32,
    save_bytes: Vec<u8>,
    random_state: Vec<i64>,
    inputs: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct UndoReplay {
    remaining: VecDeque<String>,
    queued_repeats: u32,
}

struct PendingProjectLoad {
    message_id: u64,
    remaining_metadata: BTreeSet<String>,
    queued_metadata: VecDeque<(String, [u8; 32])>,
    candidate: PendingProjectCandidate,
}

enum PendingProjectCandidate {
    Cold(PendingColdProjectLoad),
    Reload(PendingProjectReload),
}

impl PendingProjectCandidate {
    fn build_mut(&mut self) -> &mut crate::project::ProjectBuild {
        match self {
            Self::Cold(candidate) => &mut candidate.build,
            Self::Reload(candidate) => &mut candidate.build,
        }
    }
}

struct PendingColdProjectLoad {
    build: crate::project::ProjectBuild,
    previous_phase: RuntimePhase,
    compiled_project_cache: Option<Arc<Vec<u8>>>,
}

struct PreparedOrdinaryLoad {
    prepared: PreparedRuntimeState,
    opaque_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
}

struct PendingProjectReload {
    build: crate::project::ProjectBuild,
    previous_phase: RuntimePhase,
    replay_origin: Option<ReplayOrigin>,
}

struct PendingConfigurationUpdate {
    preparation_message_id: u64,
    project_revision: u64,
    expected_source_digest: ProtocolBytes,
    prepared_source_digest: ProtocolBytes,
    contents: String,
    values: era_config::ConfigStore,
    document: era_config::ReraConfigDocument,
    changed_codes: BTreeSet<String>,
}

#[allow(clippy::struct_excessive_bools)]
struct PendingCandidateCommit {
    state: PreparedCandidateState,
    presentation: PresentationModel,
    project_snapshot: Option<NormalizedProjectSnapshot>,
    message_skip: bool,
    skip_print: bool,
    user_defined_skip: bool,
    saved_skip: bool,
    force_kana_mode: u8,
    effects: Vec<EffectKind>,
    save_bytes: Vec<u8>,
    save_slot: Option<u32>,
}
