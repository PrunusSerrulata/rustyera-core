use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use era_debug_protocol::{DebugMessage, DebugScope, GrantToken};
use era_protocol::{
    Channel, ProtocolBytes, ProtocolError, ProtocolVersion, SessionEpoch, SessionId, VersionRange,
    WireLimits, decode_canonical, decode_envelope, encode_canonical, encode_envelope,
    negotiate_version,
};
use era_runtime_protocol::{
    AdvanceTime, AudioEffect, AudioEffectAction, CancelExternalRequest, CellAlignment,
    ClientCapabilities, ClientHello, CommandErrorCode, CommandRejected, DiagnosticSeverity,
    EffectAcknowledgement, EffectBatch, EffectEvent, EffectKind, EffectOutcomeStatus, ExitReason,
    ExitRequested, ExternalRequestKind, FaultCode, FrontendInput, FrontendIoErrorKind,
    GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest,
    GetKeyStateResponse, IMAGE_METADATA_OPERATION, IMAGE_METADATA_OPERATION_VERSION,
    IMAGE_PIXEL_OPERATION, IMAGE_PIXEL_OPERATION_VERSION, ImageMetadataRequest,
    ImageMetadataResponse, ImagePixelRequest, ImagePixelResponse, InputIntent, InputWait,
    InteractionToken, LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION, LineAlignment,
    LocalDateTimeRequest, LocalDateTimeResponse, OPEN_URL_OPERATION, OPEN_URL_OPERATION_VERSION,
    OpenUrlRequest, OpenUrlResponse, ProjectLoadReport, ProjectManifest, ProtocolDiagnostic,
    RANDOM_SEED_OPERATION, RANDOM_SEED_OPERATION_VERSION, RUNTIME_PROTOCOL_VERSION,
    RandomSeedRequest, RandomSeedResponse, ReloadProject, RuntimeFault, RuntimeFeature,
    RuntimeLimits, RuntimeMessage, RuntimePhase, RuntimeResynchronized, RuntimeStateChanged,
    ServerHello, ServiceCapability, ServiceKind, ServiceRequest, ServiceResponse, ServiceResult,
    ShutdownReady, SnapshotIneligibleReason, StartMode, StartRequest, StateExportChunk,
    StateExportChunkRequest, StateExportKind, StateExportReady, StateExportRequest,
    StateExportResult, StateImportAccepted, StateImportBegin, StateImportChunk, StateImportCommit,
    StateImportReady, StateTransferCancel, StateTransferDescriptor, StorageCapabilities,
    StorageEntry, StorageNamespace, StorageOperation, StoragePrecondition, StorageRequest,
    StorageResponse, StorageResult, SystemTextArgument, SystemTextKey, UPDATE_CHECK_OPERATION,
    UPDATE_CHECK_OPERATION_VERSION, UpdateCheckRequest, UpdateCheckResponse, VersionRejected,
    WaitChange, WaitKind, WaitStability,
};
use erabasic_compiler::IncrementalState;
use erabasic_validator::ValidatedArtifact;
use erabasic_vm::{
    EraSaveScope, EraState, HostReady, HostWaitStability, HostWrite, PlaceDescriptor,
    PreparedCandidateState, RunBudget, RuntimeVm, StructuredScope, VmConfig, VmDriveMode,
    VmHostCompletion, VmHostRequest, VmPortEvent, VmPortStop, VmRestorePort, VmRuntimeFill,
    VmRuntimePort, VmRuntimeStatePort, VmRuntimeStateTransaction, VmRuntimeWrite, VmSnapshot,
    VmValue,
};
use serde::{Deserialize, Serialize};

use crate::controller::{SystemController, SystemFlow, SystemStep};
use crate::host::{ClockOperation, ExternalCompletion, PendingInput, PostInputAction, input_wait};
use crate::operation::{
    CandidateSaveContinuation, PendingOperations, PendingService, PendingStorage,
};
use crate::presentation::{PresentationModel, display_value, logical_line_string};
use crate::project::{NormalizedProjectSnapshot, apply_project_delta, build_project};
use crate::runtime_snapshot::{
    self, CULTURE_TABLE_VERSION, RUNTIME_SNAPSHOT_FORMAT_VERSION, RuntimeSnapshotPayload,
};
use crate::save_adapter::{
    decode_era_save, decode_scoped_save, encode_era_save, encode_scoped_save,
    merge_opaque_extensions, merge_structured_extensions,
};

mod debug_session;

#[derive(Clone, Copy, Debug)]
pub struct RuntimeOptions {
    pub session_id: SessionId,
    pub limits: RuntimeLimits,
    pub wire_limits: WireLimits,
    pub vm_config: VmConfig,
    /// Creator-owned upper bound for [`DebugScope`] discriminants.
    pub debug_scope_mask: u64,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            session_id: SessionId { high: 0, low: 1 },
            limits: RuntimeLimits {
                maximum_envelope_bytes: 16 * 1024 * 1024,
                maximum_payload_bytes: 15 * 1024 * 1024,
                maximum_pending_requests: 1024,
                maximum_journal_entries: 4096,
                maximum_drive_instructions: 100_000,
                maximum_transfer_bytes: 1024 * 1024 * 1024,
            },
            wire_limits: WireLimits::default(),
            vm_config: VmConfig::default(),
            debug_scope_mask: 0,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeDriveReport {
    pub state: RuntimeDriveState,
    pub vm_instructions: u64,
    pub runtime_transitions: u32,
    pub queued_envelopes: u32,
}

#[derive(Debug)]
pub enum RuntimeError {
    Protocol(ProtocolError),
    InvalidSequence { expected: u64, actual: u64 },
    SessionMismatch,
    ResourceLimit(&'static str),
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
            Self::ResourceLimit(message) => formatter.write_str(message),
            Self::Internal(message) => formatter.write_str(message),
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
    committed: bool,
}

#[derive(Debug)]
struct OutboundStateTransfer {
    descriptor: StateTransferDescriptor,
    bytes: Vec<u8>,
    next_offset: u64,
}

#[derive(Debug)]
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
    inbound: VecDeque<(u64, InboundMessage)>,
    outbound: VecDeque<Vec<u8>>,
    outbound_journal: BTreeMap<u64, Vec<u8>>,
    effect_journal: BTreeMap<u64, EffectEvent>,
    accepted_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    accepted_debug_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    active_debug_grant: Option<ActiveDebugGrant>,
    next_debug_grant_id: u64,
    debug_resume_phase: Option<RuntimePhase>,
    debug_frontend_time_sample: Option<u64>,
    artifact: Option<ValidatedArtifact>,
    incremental: IncrementalState,
    vm: Option<RuntimeVm>,
    presentation: PresentationModel,
    operations: PendingOperations,
    key_toggle_state: [u8; 256],
    message_skip: bool,
    skip_print: bool,
    user_defined_skip: bool,
    saved_skip: bool,
    client_focused: bool,
    client_audio_available: bool,
    command_intents: BTreeMap<InteractionToken, VmValue>,
    reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    exit_requested: Option<ExitRequested>,
    controller: SystemController,
    project_snapshot: Option<NormalizedProjectSnapshot>,
    selected_locale: String,
    available_fonts: BTreeSet<String>,
    service_capabilities: BTreeMap<(ServiceKind, String), ProtocolVersion>,
    storage_capabilities: StorageCapabilities,
    save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    system_menu: SystemMenuState,
    load_slot_paths: Vec<String>,
    occupied_slot_paths: BTreeSet<String>,
    slot_revisions: BTreeMap<String, String>,
    slot_labels: BTreeMap<String, String>,
    invalid_slot_paths: BTreeSet<String>,
    system_menu_host_request: Option<erabasic_vm::HostRequestId>,
    system_menu_page: u32,
    inbound_transfer: Option<InboundStateTransfer>,
    outbound_transfer: Option<OutboundStateTransfer>,
    pending_project_load: Option<PendingProjectLoad>,
    pending_candidate_commit: Option<PendingCandidateCommit>,
    candidate_clock: Option<LocalDateTimeResponse>,
}

struct PendingProjectLoad {
    message_id: u64,
    report: ProjectLoadReport,
    remaining_metadata: BTreeSet<String>,
    reload: Option<PendingProjectReload>,
}

struct PendingProjectReload {
    build: crate::project::ProjectBuild,
    previous_phase: RuntimePhase,
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
    effects: Vec<EffectKind>,
}

impl RuntimeSession {
    #[must_use]
    pub fn new(options: RuntimeOptions) -> Self {
        Self {
            options,
            state: SessionState::Negotiating,
            phase: RuntimePhase::Negotiating,
            revision: 0,
            epoch: SessionEpoch(0),
            expected_inbound_sequence: 0,
            expected_debug_sequence: 0,
            outbound_sequence: 0,
            debug_outbound_sequence: 0,
            next_message_id: 1,
            next_request_id: 1,
            next_wait_id: 1,
            next_interaction_id: 1,
            next_transfer_id: 1,
            next_effect_id: 1,
            logical_time_ns: 0,
            frontend_time_origin: None,
            random_seed: None,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            outbound_journal: BTreeMap::new(),
            effect_journal: BTreeMap::new(),
            accepted_message_ids: BTreeMap::new(),
            accepted_debug_message_ids: BTreeMap::new(),
            active_debug_grant: None,
            next_debug_grant_id: 1,
            debug_resume_phase: None,
            debug_frontend_time_sample: None,
            artifact: None,
            incremental: IncrementalState::default(),
            vm: None,
            presentation: PresentationModel::default(),
            operations: PendingOperations::default(),
            key_toggle_state: [0; 256],
            message_skip: false,
            skip_print: false,
            user_defined_skip: false,
            saved_skip: false,
            client_focused: true,
            client_audio_available: true,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            exit_requested: None,
            controller: SystemController::default(),
            project_snapshot: None,
            selected_locale: "ja".into(),
            available_fonts: BTreeSet::new(),
            service_capabilities: BTreeMap::new(),
            storage_capabilities: StorageCapabilities {
                revisions: false,
                atomic_replace: false,
                missing_precondition: false,
                delete: false,
            },
            save_extensions: Vec::new(),
            system_menu: SystemMenuState::Title,
            load_slot_paths: Vec::new(),
            occupied_slot_paths: BTreeSet::new(),
            slot_revisions: BTreeMap::new(),
            slot_labels: BTreeMap::new(),
            invalid_slot_paths: BTreeSet::new(),
            system_menu_host_request: None,
            system_menu_page: 0,
            inbound_transfer: None,
            outbound_transfer: None,
            pending_project_load: None,
            pending_candidate_commit: None,
            candidate_clock: None,
        }
    }

    /// Decode and queue one frontend envelope without executing runtime work.
    ///
    /// # Errors
    ///
    /// Rejects malformed, oversized, out-of-sequence, or wrong-session envelopes.
    pub fn submit_envelope(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        let envelope = decode_envelope(bytes, self.options.wire_limits)?;
        if envelope.channel == Channel::Debug && self.state != SessionState::Active {
            return Err(RuntimeError::SessionMismatch);
        }
        if self.state == SessionState::Active
            && (envelope.session != Some(self.options.session_id)
                || envelope.session_epoch != Some(self.epoch))
        {
            return Err(RuntimeError::SessionMismatch);
        }
        let envelope_hash = blake3::hash(bytes);
        let (expected_sequence, accepted_ids) = match envelope.channel {
            Channel::Runtime => (
                &mut self.expected_inbound_sequence,
                &mut self.accepted_message_ids,
            ),
            Channel::Debug => (
                &mut self.expected_debug_sequence,
                &mut self.accepted_debug_message_ids,
            ),
        };
        if envelope.sequence < *expected_sequence {
            if accepted_ids.get(&envelope.message_id) == Some(&(envelope.sequence, envelope_hash)) {
                return Ok(());
            }
            return Err(RuntimeError::InvalidSequence {
                expected: *expected_sequence,
                actual: envelope.sequence,
            });
        }
        if envelope.sequence != *expected_sequence {
            return Err(RuntimeError::InvalidSequence {
                expected: *expected_sequence,
                actual: envelope.sequence,
            });
        }
        if self.inbound.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("inbound journal is full"));
        }
        let message_id = envelope.message_id;
        let message = match envelope.channel {
            Channel::Runtime => InboundMessage::Runtime(RuntimeMessage::from_envelope(&envelope)?),
            Channel::Debug => InboundMessage::Debug(DebugMessage::from_envelope(&envelope)?),
        };
        *expected_sequence = expected_sequence.saturating_add(1);
        accepted_ids.insert(message_id, (envelope.sequence, envelope_hash));
        while accepted_ids.len() > self.options.limits.maximum_journal_entries as usize {
            accepted_ids.pop_first();
        }
        self.inbound.push_back((message_id, message));
        Ok(())
    }

    /// Execute a bounded number of actor transitions and VM instructions.
    ///
    /// # Errors
    ///
    /// Returns an error if a queued transition violates a VM or protocol invariant.
    pub fn drive(
        &mut self,
        budget: RuntimeDriveBudget,
    ) -> Result<RuntimeDriveReport, RuntimeError> {
        let transition_limit = budget.maximum_runtime_transitions.max(1);
        let mut transitions = 0;
        let mut instructions = 0;
        while transitions < transition_limit {
            if let Some((message_id, message)) = self.inbound.pop_front() {
                match message {
                    InboundMessage::Runtime(message) => self.handle_message(message_id, message)?,
                    InboundMessage::Debug(message) => {
                        self.handle_debug_message(message_id, message)?;
                    }
                }
                transitions += 1;
                continue;
            }
            if self.phase == RuntimePhase::Running && instructions < budget.maximum_vm_instructions
            {
                let remaining = budget.maximum_vm_instructions - instructions;
                let Some(mut vm) = self.vm.take() else {
                    self.fault(FaultCode::Internal, "running phase has no VM", None)?;
                    transitions += 1;
                    continue;
                };
                let report = vm.drive(
                    RunBudget {
                        maximum_instructions: remaining
                            .min(self.options.limits.maximum_drive_instructions),
                        maximum_host_calls: self.options.limits.maximum_pending_requests,
                        fiber_quantum: RunBudget::default().fiber_quantum,
                    },
                    VmDriveMode::Normal,
                );
                instructions = instructions.saturating_add(report.instructions);
                let stop = report.stop;
                for event in report.events {
                    self.handle_vm_event(&mut vm, event)?;
                }
                if self.operations.active_input().is_some()
                    && !vm.has_runnable_fibers()
                    && self.phase == RuntimePhase::Running
                {
                    self.set_phase(RuntimePhase::WaitingInput)?;
                }
                self.vm = Some(vm);
                transitions += 1;
                if self.phase != RuntimePhase::Running
                    || stop != VmPortStop::BudgetExhausted
                    || report.instructions == 0
                {
                    break;
                }
                continue;
            }
            break;
        }
        let state = if self.phase == RuntimePhase::Faulted {
            RuntimeDriveState::Faulted
        } else if self.phase == RuntimePhase::Stopped {
            RuntimeDriveState::Stopped
        } else if !self.outbound.is_empty() {
            RuntimeDriveState::OutputReady
        } else if !self.inbound.is_empty()
            || (self.phase == RuntimePhase::Running
                && instructions >= budget.maximum_vm_instructions)
        {
            RuntimeDriveState::MoreWork
        } else {
            RuntimeDriveState::Idle
        };
        Ok(RuntimeDriveReport {
            state,
            vm_instructions: instructions,
            runtime_transitions: transitions,
            queued_envelopes: u32::try_from(self.outbound.len()).unwrap_or(u32::MAX),
        })
    }

    #[must_use]
    pub fn poll_envelope(&mut self) -> Option<Vec<u8>> {
        self.outbound.pop_front()
    }

    #[must_use]
    pub const fn phase(&self) -> RuntimePhase {
        self.phase
    }

    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

    /// Revision retained for deterministic reload staging without filesystem access.
    #[must_use]
    pub fn project_revision(&self) -> Option<u64> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.manifest.project_revision)
    }

    #[must_use]
    pub fn project_sorts_filenames(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.sort_with_filename)
    }

    #[must_use]
    pub fn project_ignored_new_random(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.use_new_random_ignored)
    }

    #[must_use]
    pub fn project_auto_save(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.auto_save)
    }

    #[must_use]
    pub fn project_save_slot_count(&self) -> Option<u32> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.save_slot_count)
    }

    #[must_use]
    pub fn project_money_label(&self) -> Option<&str> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.money_label.as_str())
    }

    #[must_use]
    pub fn project_money_first(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.money_first)
    }

    #[must_use]
    pub fn project_maximum_shop_items(&self) -> Option<u32> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.maximum_shop_items)
    }

    #[allow(clippy::too_many_lines)]
    fn handle_message(
        &mut self,
        message_id: u64,
        message: RuntimeMessage,
    ) -> Result<(), RuntimeError> {
        if self.state == SessionState::Negotiating {
            return match message {
                RuntimeMessage::ClientHello(hello) => self.hello(message_id, &hello),
                _ => self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "ClientHello must be the first message",
                ),
            };
        }
        if self.phase == RuntimePhase::DebugPaused && debugger_suspends_message(&message) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "state-changing runtime messages are suspended by a debugger stop",
            );
        }
        match message {
            RuntimeMessage::ProjectManifest(manifest) => self.load_project(message_id, &manifest),
            RuntimeMessage::Start(start) => self.start(message_id, &start),
            RuntimeMessage::Input(input) => self.complete_input(message_id, input),
            RuntimeMessage::AdvanceTime(time) if self.phase == RuntimePhase::DebugPaused => {
                self.debug_frontend_time_sample = Some(time.monotonic_time_ns);
                Ok(())
            }
            RuntimeMessage::AdvanceTime(time) => self.advance_time(message_id, time),
            RuntimeMessage::DeviceStateChanged(state) => {
                if self.phase == RuntimePhase::DebugPaused {
                    self.debug_frontend_time_sample = Some(state.monotonic_time_ns);
                } else {
                    self.observe_frontend_time(state.monotonic_time_ns);
                }
                Ok(())
            }
            RuntimeMessage::ClientStateChanged(state) => {
                self.client_focused = state.focused;
                self.client_audio_available = state.audio_available;
                Ok(())
            }
            RuntimeMessage::EffectAcknowledgement(acknowledgement) => {
                self.acknowledge_effects(message_id, acknowledgement)
            }
            RuntimeMessage::ServiceResponse(response) => {
                self.complete_service(message_id, response)
            }
            RuntimeMessage::StateExportRequest(request) => self.export_state(message_id, request),
            RuntimeMessage::StateImportBegin(request) => {
                self.begin_state_import(message_id, request)
            }
            RuntimeMessage::StateImportChunk(chunk) => self.append_state_import(message_id, &chunk),
            RuntimeMessage::StateImportCommit(commit) => {
                self.commit_state_import(message_id, commit)
            }
            RuntimeMessage::StateExportChunkRequest(request) => {
                self.read_state_export(message_id, request)
            }
            RuntimeMessage::StateTransferCancel(cancel) => {
                self.cancel_state_transfer(message_id, cancel)
            }
            RuntimeMessage::ReloadProject(reload) => self.reload_project(message_id, &reload),
            RuntimeMessage::ShutdownRequest(_) => self.shutdown(message_id),
            RuntimeMessage::Acknowledge(ack) => {
                self.outbound_journal
                    .retain(|sequence, _| *sequence > ack.through_sequence);
                Ok(())
            }
            RuntimeMessage::Resynchronize(_) => self.resynchronize(message_id),
            RuntimeMessage::StorageResponse(response) => {
                self.complete_storage(message_id, response)
            }
            RuntimeMessage::ClientHello(_)
            | RuntimeMessage::ServerHello(_)
            | RuntimeMessage::VersionRejected(_)
            | RuntimeMessage::ProjectLoadReport(_)
            | RuntimeMessage::StateChanged(_)
            | RuntimeMessage::ExitRequested(_)
            | RuntimeMessage::WaitChanged(_)
            | RuntimeMessage::PresentationSnapshot(_)
            | RuntimeMessage::PresentationDelta(_)
            | RuntimeMessage::EffectBatch(_)
            | RuntimeMessage::StorageRequest(_)
            | RuntimeMessage::ServiceRequest(_)
            | RuntimeMessage::CancelExternalRequest(_)
            | RuntimeMessage::StateExportReady(_)
            | RuntimeMessage::StateImportAccepted(_)
            | RuntimeMessage::StateImportReady(_)
            | RuntimeMessage::StateExportChunk(_)
            | RuntimeMessage::ShutdownReady(_)
            | RuntimeMessage::Fault(_)
            | RuntimeMessage::CommandRejected(_)
            | RuntimeMessage::RuntimeResynchronized(_)
            | RuntimeMessage::Diagnostic(_) => self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "message direction is frontend-incompatible",
            ),
        }
    }

    fn hello(&mut self, message_id: u64, hello: &ClientHello) -> Result<(), RuntimeError> {
        let supported = VersionRange::exact(RUNTIME_PROTOCOL_VERSION);
        let Some(selected) = negotiate_version(hello.runtime_versions, supported) else {
            return self.emit(
                RuntimeMessage::VersionRejected(VersionRejected {
                    supported,
                    message: "runtime protocol 13.0 is required".into(),
                }),
                Some(message_id),
            );
        };
        self.state = SessionState::Active;
        self.epoch = SessionEpoch(1);
        let limits = intersect_limits(self.options.limits, hello.requested_limits);
        self.options.limits = limits;
        self.options.wire_limits.maximum_envelope_bytes =
            usize::try_from(limits.maximum_envelope_bytes).unwrap_or(usize::MAX);
        self.options.wire_limits.maximum_payload_bytes =
            usize::try_from(limits.maximum_payload_bytes).unwrap_or(usize::MAX);
        let implemented = [
            RuntimeFeature::TraditionalSave,
            RuntimeFeature::VmSnapshot,
            RuntimeFeature::ProjectReload,
            RuntimeFeature::Storage,
            RuntimeFeature::TimedInput,
            RuntimeFeature::ExternalServices,
            RuntimeFeature::StateResynchronization,
        ];
        let features = implemented
            .into_iter()
            .filter(|feature| hello.features.contains(feature))
            .collect();
        let selected_capabilities = selected_capabilities(&hello.capabilities);
        self.service_capabilities = selected_capabilities
            .services
            .iter()
            .map(|capability| {
                (
                    (capability.kind, capability.operation.clone()),
                    capability.versions.maximum,
                )
            })
            .collect();
        self.storage_capabilities = selected_capabilities.storage;
        self.available_fonts = selected_capabilities
            .available_fonts
            .iter()
            .map(|name| name.to_lowercase())
            .collect();
        self.selected_locale = select_locale(&hello.preferred_locales).into();
        self.presentation.set_projection(
            selected_capabilities.column_cells,
            selected_capabilities.separators,
            selected_capabilities.html,
            selected_capabilities.graphics,
            selected_capabilities.audio,
        );
        self.emit(
            RuntimeMessage::ServerHello(ServerHello {
                selected_version: selected,
                session: self.options.session_id,
                features,
                limits,
                epoch: self.epoch.0,
                selected_capabilities,
                selected_locale: self.selected_locale.clone(),
            }),
            Some(message_id),
        )
    }

    fn load_project(
        &mut self,
        message_id: u64,
        manifest: &ProjectManifest,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::Ready | RuntimePhase::Faulted
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project loading requires an idle runtime",
            );
        }
        self.set_phase(RuntimePhase::LoadingProject)?;
        let mut build = build_project(manifest, Some(&self.incremental));
        self.incremental = build.incremental;
        self.artifact = build.artifact;
        self.project_snapshot = build.snapshot;
        let metadata = self
            .project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !build.report.success || metadata.is_empty() {
            return self.finish_project_load(message_id, build.report);
        }
        if self
            .service_capabilities
            .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
            != Some(&IMAGE_METADATA_OPERATION_VERSION)
        {
            build.report.success = false;
            build.report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.missing_image_metadata_service".into(),
                severity: DiagnosticSeverity::Error,
                message: "resource sprites require the negotiated image_metadata service".into(),
                source: None,
            });
            return self.finish_project_load(message_id, build.report);
        }
        let remaining_metadata = metadata
            .iter()
            .map(|(path, _)| path.to_ascii_lowercase())
            .collect();
        self.pending_project_load = Some(PendingProjectLoad {
            message_id,
            report: build.report,
            remaining_metadata,
            reload: None,
        });
        for (relative_path, digest) in metadata {
            let request_id = self.allocate_request()?;
            self.operations.insert_service(
                request_id,
                PendingService::ProjectImageMetadata {
                    relative_path: relative_path.clone(),
                },
            );
            self.emit(
                RuntimeMessage::ServiceRequest(ServiceRequest {
                    request_id,
                    kind: ServiceKind::Image,
                    operation: IMAGE_METADATA_OPERATION.into(),
                    operation_version: IMAGE_METADATA_OPERATION_VERSION,
                    payload: ProtocolBytes::new(encode_canonical(&ImageMetadataRequest {
                        resource_id: relative_path,
                        content_digest: ProtocolBytes::new(digest),
                    })?),
                    deadline_ns: None,
                }),
                None,
            )?;
        }
        Ok(())
    }

    fn finish_project_load(
        &mut self,
        message_id: u64,
        report: ProjectLoadReport,
    ) -> Result<(), RuntimeError> {
        if report.success {
            if let Some(snapshot) = &self.project_snapshot {
                self.presentation.configure_layout(
                    snapshot.viewport_width,
                    snapshot.print_c_per_line,
                    snapshot.print_c_length,
                );
            }
            self.sync_resource_replay();
        } else {
            self.artifact = None;
            self.project_snapshot = None;
        }
        let success = report.success;
        self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id))?;
        self.set_phase(if success {
            RuntimePhase::Ready
        } else {
            RuntimePhase::Faulted
        })
    }

    #[allow(clippy::too_many_lines)]
    fn reload_project(
        &mut self,
        message_id: u64,
        reload: &ReloadProject,
    ) -> Result<(), RuntimeError> {
        let previous_phase = self.phase;
        if !matches!(
            previous_phase,
            RuntimePhase::Ready | RuntimePhase::Running | RuntimePhase::WaitingInput
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project reload requires a ready or running runtime",
            );
        }
        if self.operations.total_count() != 0 && !self.operations.is_snapshot_stable() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project reload cannot cross transient runtime operations",
            );
        }
        let current = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("project reload has no base manifest".into()))?;
        let manifest = match apply_project_delta(&current.manifest, reload) {
            Ok(manifest) => manifest,
            Err(error) => {
                return self.reject(message_id, CommandErrorCode::InvalidValue, &error);
            }
        };
        self.set_phase(RuntimePhase::Reloading)?;
        let mut build = build_project(&manifest, Some(&self.incremental));
        if !build.report.success {
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        if let (Some(next), Some(previous)) =
            (build.snapshot.as_mut(), self.project_snapshot.as_ref())
        {
            next.resource_graph
                .inherit_runtime_graph(&previous.resource_graph);
        }
        let metadata = build
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !metadata.is_empty() {
            if self
                .service_capabilities
                .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
                != Some(&IMAGE_METADATA_OPERATION_VERSION)
            {
                build.report.success = false;
                build.report.diagnostics.push(ProtocolDiagnostic {
                    code: "runtime.missing_image_metadata_service".into(),
                    severity: DiagnosticSeverity::Error,
                    message:
                        "changed image resources require the negotiated image_metadata service"
                            .into(),
                    source: None,
                });
                self.emit(
                    RuntimeMessage::ProjectLoadReport(build.report),
                    Some(message_id),
                )?;
                return self.set_phase(previous_phase);
            }
            let remaining_metadata = metadata
                .iter()
                .map(|(path, _)| path.to_ascii_lowercase())
                .collect();
            let report = build.report.clone();
            self.pending_project_load = Some(PendingProjectLoad {
                message_id,
                report,
                remaining_metadata,
                reload: Some(PendingProjectReload {
                    build,
                    previous_phase,
                }),
            });
            for (relative_path, digest) in metadata {
                let request_id = self.allocate_request()?;
                self.operations.insert_service(
                    request_id,
                    PendingService::ProjectImageMetadata {
                        relative_path: relative_path.clone(),
                    },
                );
                self.emit(
                    RuntimeMessage::ServiceRequest(ServiceRequest {
                        request_id,
                        kind: ServiceKind::Image,
                        operation: IMAGE_METADATA_OPERATION.into(),
                        operation_version: IMAGE_METADATA_OPERATION_VERSION,
                        payload: ProtocolBytes::new(encode_canonical(&ImageMetadataRequest {
                            resource_id: relative_path,
                            content_digest: ProtocolBytes::new(digest),
                        })?),
                        deadline_ns: None,
                    }),
                    None,
                )?;
            }
            return Ok(());
        }
        self.commit_project_reload(message_id, build, previous_phase)
    }

    fn commit_project_reload(
        &mut self,
        message_id: u64,
        mut build: crate::project::ProjectBuild,
        previous_phase: RuntimePhase,
    ) -> Result<(), RuntimeError> {
        let target = build
            .artifact
            .take()
            .ok_or_else(|| RuntimeError::Internal("successful reload has no artifact".into()))?;
        if let Some(vm) = &mut self.vm
            && let Err(error) = vm.prepare_hot_reload(target.clone())
        {
            build.report.success = false;
            build.report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.hot_reload_incompatible".into(),
                severity: DiagnosticSeverity::Error,
                message: error.to_string(),
                source: None,
            });
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        if let Some(vm) = &mut self.vm {
            vm.commit_hot_reload()
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }

        self.artifact = Some(target);
        self.incremental = build.incremental;
        self.project_snapshot = build.snapshot;
        if let Some(snapshot) = &self.project_snapshot {
            self.presentation.configure_layout(
                snapshot.viewport_width,
                snapshot.print_c_per_line,
                snapshot.print_c_length,
            );
        }
        self.sync_resource_replay();
        let new_epoch = self.epoch.0.saturating_add(1);
        let (tokens, waits) = self.operations.rebind_stable_inputs(
            new_epoch,
            &mut self.next_wait_id,
            &mut self.next_interaction_id,
        );
        self.presentation.rebind_interactions(&tokens, &waits);
        self.command_intents = std::mem::take(&mut self.command_intents)
            .into_iter()
            .filter_map(|(old, value)| tokens.get(&old).copied().map(|new| (new, value)))
            .collect();
        self.reusable_system_intents = std::mem::take(&mut self.reusable_system_intents)
            .into_iter()
            .filter_map(|(old, value)| tokens.get(&old).copied().map(|new| (new, value)))
            .collect();
        self.epoch = SessionEpoch(new_epoch);
        self.accepted_message_ids.clear();
        self.accepted_debug_message_ids.clear();
        self.emit(
            RuntimeMessage::ProjectLoadReport(build.report),
            Some(message_id),
        )?;
        self.set_phase(previous_phase)?;
        self.renew_debug_grant()?;
        self.emit_presentation()
    }

    fn start(&mut self, message_id: u64, request: &StartRequest) -> Result<(), RuntimeError> {
        if self.phase != RuntimePhase::Ready {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "start requires a successfully loaded project",
            );
        }
        if matches!(request.mode, StartMode::NewGame { .. }) {
            self.advance_epoch();
        }
        match request.mode {
            StartMode::NewGame { seed: Some(seed) } => self.start_new_game(seed),
            StartMode::NewGame { seed: None } => {
                self.set_phase(RuntimePhase::Starting)?;
                let request_id = self.allocate_request()?;
                self.operations
                    .insert_service(request_id, PendingService::StartEntropy);
                self.emit(
                    RuntimeMessage::ServiceRequest(ServiceRequest {
                        request_id,
                        kind: ServiceKind::Entropy,
                        operation: RANDOM_SEED_OPERATION.into(),
                        operation_version: RANDOM_SEED_OPERATION_VERSION,
                        payload: ProtocolBytes::new(encode_canonical(&RandomSeedRequest {})?),
                        deadline_ns: None,
                    }),
                    Some(message_id),
                )
            }
            StartMode::TraditionalSave { transfer_id } => {
                let Some(bytes) = self.consume_state_import(
                    message_id,
                    transfer_id,
                    StateExportKind::TraditionalSave,
                )?
                else {
                    return Ok(());
                };
                self.start_traditional_save(message_id, &bytes)
            }
            StartMode::VmSnapshot { transfer_id } => {
                let Some(bytes) = self.consume_state_import(
                    message_id,
                    transfer_id,
                    StateExportKind::VmSnapshot,
                )?
                else {
                    return Ok(());
                };
                self.start_vm_snapshot(message_id, &bytes)
            }
        }
    }

    fn start_traditional_save(
        &mut self,
        message_id: u64,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let artifact = self
            .artifact
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let decoded = match decode_era_save(bytes, artifact.artifact()) {
            Ok(decoded) => decoded,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("traditional save is invalid: {error}"),
                );
            }
        };
        let mut vm = RuntimeVm::new(
            self.artifact
                .clone()
                .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?,
            self.options.vm_config,
        );
        let version = decoded.state.version;
        let description = decoded.description.clone();
        let prepared = match vm.prepare_runtime_state_with_extensions(
            VmRuntimeStateTransaction::RestoreOrdinary(Box::new(decoded.state)),
            StructuredScope::Ordinary,
            &decoded.structured_extensions,
        ) {
            Ok((prepared, _)) => prepared,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("traditional save is incompatible: {error}"),
                );
            }
        };
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let last_load = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::SetLastLoad {
                version,
                slot: -1,
                text: description,
            })
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(last_load)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions = decoded.opaque_extensions;
        if let Some(project) = &mut self.project_snapshot {
            project.resource_graph.reset_runtime_graph();
        }
        self.sync_resource_replay();
        self.advance_epoch();
        self.set_phase(RuntimePhase::Starting)?;
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        if self.controller.prepare_load_sequence(vm.vm().artifact()) {
            self.spawn_next_event(&mut vm)?;
        } else {
            self.continue_system_flow(&mut vm)?;
        }
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        self.renew_debug_grant()
    }

    #[allow(clippy::too_many_lines)]
    fn start_vm_snapshot(&mut self, message_id: u64, bytes: &[u8]) -> Result<(), RuntimeError> {
        let maximum =
            usize::try_from(self.options.limits.maximum_transfer_bytes).unwrap_or(usize::MAX);
        let payload = match runtime_snapshot::decode(bytes, maximum) {
            Ok(payload) => payload,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("runtime snapshot is invalid: {error}"),
                );
            }
        };
        let artifact = self
            .artifact
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let project = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded project identity is missing".into()))?;
        if payload.artifact_id != artifact.artifact().manifest.artifact_id
            || payload.project_identity != project.project_identity
            || payload.resource_count != u64::try_from(project.resources.len()).unwrap_or(u64::MAX)
            || payload.selected_locale != self.selected_locale
            || payload.culture_table_version != CULTURE_TABLE_VERSION
            || !payload.operations.is_snapshot_stable()
        {
            return self.reject(
                message_id,
                CommandErrorCode::VersionMismatch,
                "runtime snapshot does not match the exact project or stable-wait contract",
            );
        }
        let system_menu = match payload.system_menu {
            0 => SystemMenuState::Title,
            1 => SystemMenuState::LoadSlots,
            2 => SystemMenuState::SaveSlots,
            3 => SystemMenuState::ConfirmOverwrite {
                slot: payload.system_menu_slot.ok_or_else(|| {
                    RuntimeError::Internal("runtime snapshot overwrite menu lacks its slot".into())
                })?,
            },
            _ => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "runtime snapshot contains an unknown system menu",
                );
            }
        };
        let vm_snapshot = match VmSnapshot::decode(&payload.vm_snapshot, maximum) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("VM snapshot is invalid: {error}"),
                );
            }
        };
        let prepared =
            match RuntimeVm::prepare_restore(artifact.clone(), self.options.vm_config, vm_snapshot)
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    return self.reject(
                        message_id,
                        CommandErrorCode::InvalidValue,
                        &format!("VM snapshot cannot be restored: {error}"),
                    );
                }
            };
        let mut expected_requests = payload.operations.input_host_requests();
        expected_requests.sort();
        let mut rebound_requests = RuntimeVm::restore_waits(&prepared)
            .iter()
            .map(|wait| wait.request)
            .collect::<Vec<_>>();
        rebound_requests.sort();
        if expected_requests != rebound_requests {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "runtime and VM snapshot waits do not correspond",
            );
        }
        let vm = RuntimeVm::commit_restore(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;

        let new_epoch = self.epoch.0.max(payload.epoch).saturating_add(1);
        let mut operations = payload.operations;
        self.next_wait_id = 1;
        self.next_interaction_id = 1;
        let (tokens, waits) = operations.rebind_stable_inputs(
            new_epoch,
            &mut self.next_wait_id,
            &mut self.next_interaction_id,
        );
        let mut presentation = payload.presentation;
        presentation.rebind_interactions(&tokens, &waits);
        let remap_intents = |values: std::collections::BTreeMap<InteractionToken, VmValue>| {
            values
                .into_iter()
                .filter_map(|(token, value)| tokens.get(&token).copied().map(|new| (new, value)))
                .collect()
        };

        self.epoch = SessionEpoch(new_epoch);
        self.accepted_message_ids.clear();
        self.vm = Some(vm);
        self.presentation = presentation;
        self.operations = operations;
        self.project_snapshot
            .as_mut()
            .expect("project identity was checked above")
            .resource_graph = payload.resource_graph;
        self.controller = payload.controller;
        self.logical_time_ns = payload.logical_time_ns;
        self.frontend_time_origin = None;
        self.random_seed = payload.random_seed;
        self.message_skip = payload.message_skip;
        self.skip_print = payload.skip_print;
        self.user_defined_skip = payload.user_defined_skip;
        self.saved_skip = payload.saved_skip;
        self.command_intents = remap_intents(payload.command_intents);
        self.reusable_system_intents = remap_intents(payload.reusable_system_intents);
        self.save_extensions = payload.save_extensions;
        self.system_menu = system_menu;
        self.load_slot_paths = payload.load_slot_paths;
        self.occupied_slot_paths = payload.occupied_slot_paths;
        self.slot_revisions.clear();
        self.slot_labels.clear();
        self.invalid_slot_paths.clear();
        self.system_menu_host_request = payload.system_menu_host_request;
        self.system_menu_page = payload.system_menu_page;
        if matches!(
            self.system_menu,
            SystemMenuState::LoadSlots | SystemMenuState::SaveSlots
        ) {
            let save = self.system_menu == SystemMenuState::SaveSlots;
            self.operations.clear();
            return self.issue_storage(
                if save {
                    PendingStorage::ListSaveSlots
                } else {
                    PendingStorage::ListLoadSlots
                },
                StorageNamespace::Save,
                StorageOperation::List {
                    pattern: Some("save*.sav".into()),
                    recursive: false,
                },
                String::new(),
            );
        }
        self.set_phase(RuntimePhase::WaitingInput)?;
        self.renew_debug_grant()?;
        self.emit_presentation()
    }

    fn start_new_game(&mut self, seed: u64) -> Result<(), RuntimeError> {
        self.random_seed = Some(seed);
        self.frontend_time_origin = None;
        if let Some(project) = &mut self.project_snapshot {
            project.resource_graph.reset_runtime_graph();
        }
        self.sync_resource_replay();
        self.set_phase(RuntimePhase::Starting)?;
        let artifact = self
            .artifact
            .take()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let title = artifact
            .artifact()
            .project_data
            .static_data
            .game_base
            .title
            .clone();
        self.presentation.set_title(title);
        let mut vm = RuntimeVm::new_with_seed(artifact, self.options.vm_config, seed);
        let prepared = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.controller.flow = Some(SystemFlow::Title);
        let result = if self
            .controller
            .prepare_function(vm.vm().artifact(), "SYSTEM_TITLE")
        {
            self.spawn_next_event(&mut vm)?;
            self.vm = Some(vm);
            self.set_phase(RuntimePhase::Running)
        } else {
            self.vm = Some(vm);
            self.open_title_menu()
        };
        result?;
        self.renew_debug_grant()
    }

    fn open_title_menu(&mut self) -> Result<(), RuntimeError> {
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths.clear();
        self.occupied_slot_paths.clear();
        self.slot_revisions.clear();
        self.slot_labels.clear();
        self.invalid_slot_paths.clear();
        self.system_menu_host_request = None;
        self.system_menu_page = 0;
        let start_token = self.allocate_interaction();
        let load_token = self.allocate_interaction();
        let submission_token = self.allocate_interaction();
        self.presentation.append_system_button(
            localized_system_text(&self.selected_locale, SystemTextKey::NewGame),
            SystemTextKey::NewGame,
            Vec::new(),
            start_token,
        );
        self.presentation.append_system_button(
            localized_system_text(&self.selected_locale, SystemTextKey::LoadGame),
            SystemTextKey::LoadGame,
            Vec::new(),
            load_token,
        );
        let wait = self.system_wait(submission_token);
        self.open_wait(
            PendingInput {
                host_request: self.system_menu_host_request,
                wait,
                result_name: None,
                choices: BTreeMap::from([
                    (start_token, VmValue::Integer(0)),
                    (load_token, VmValue::Integer(1)),
                ]),
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    fn system_wait(&mut self, submission_token: InteractionToken) -> InputWait {
        InputWait {
            wait_id: self.allocate_wait(),
            kind: WaitKind::IntegerButton,
            stability: WaitStability::StableInput,
            one_input: false,
            stop_message_skip: false,
            system_input: true,
            mouse_input: false,
            default_value: None,
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token,
            countdown_remaining_ms: None,
        }
    }

    fn handle_vm_event(
        &mut self,
        vm: &mut RuntimeVm,
        event: VmPortEvent,
    ) -> Result<(), RuntimeError> {
        match event {
            VmPortEvent::HostCall(request) => self.handle_host_call(vm, &request),
            VmPortEvent::FiberFaulted(_, fault) => self.fault(
                FaultCode::VmFault,
                &fault.message,
                Some(erabasic_vm::VmExecutionOrigin {
                    generation: fault.generation,
                    function: fault.function,
                    function_name: fault.function_name,
                    instruction: fault.instruction,
                    command: fault.command,
                    source: fault.source,
                }),
            ),
            VmPortEvent::FiberCompleted(fiber, value) => {
                if self.controller.completed(fiber, value.as_ref()) {
                    self.spawn_next_event(vm)?;
                    if self.controller.is_complete()
                        && matches!(
                            self.controller.flow,
                            Some(
                                SystemFlow::Title
                                    | SystemFlow::First
                                    | SystemFlow::AfterTrain
                                    | SystemFlow::TurnEnd
                                    | SystemFlow::Normal
                            )
                        )
                    {
                        self.controller.flow = Some(SystemFlow::Normal);
                        return self.fault(
                            FaultCode::VmFault,
                            "script execution ended while the reference system was in NORMAL",
                            None,
                        );
                    }
                    if self.controller.is_complete() && self.controller.step != SystemStep::None {
                        return self.continue_system_flow(vm);
                    }
                }
                Ok(())
            }
            VmPortEvent::FiberYielded(_) => Ok(()),
            VmPortEvent::DebugStopped(stop) => self.enter_debug_stop(stop, None),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_host_call(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        if let Some(time) = self.candidate_clock {
            match request.import.contract.candidate {
                erabasic_bytecode::CandidatePolicy::Forbidden => {
                    return Err(RuntimeError::Internal(format!(
                        "{} is forbidden during candidate SAVEINFO execution",
                        request.import.import.name
                    )));
                }
                erabasic_bytecode::CandidatePolicy::FrozenClock => {
                    return complete_frozen_clock(vm, request, time);
                }
                erabasic_bytecode::CandidatePolicy::ReadOnly
                | erabasic_bytecode::CandidatePolicy::CloneCommit
                | erabasic_bytecode::CandidatePolicy::BufferedEffect => {}
            }
        }
        let name = request.import.import.name.to_ascii_uppercase();
        if name == "SKIPDISP" {
            self.skip_print = integer_argument_value(&request.arguments, 0)? != 0;
            self.user_defined_skip = self.skip_print;
            let writes = self.result_write(i64::from(self.skip_print))?;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            );
        }
        if name == "SKIPLOG" {
            self.message_skip = integer_argument_value(&request.arguments, 0)? != 0;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "NOSKIP" {
            self.saved_skip = self.skip_print;
            self.skip_print = false;
            return commit_integer_result(vm, request.id, 1);
        }
        if name == "ENDNOSKIP" {
            if self.saved_skip {
                self.skip_print = true;
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if matches!(
            name.as_str(),
            "ISSKIP" | "MESSKIP" | "MOUSESKIP" | "LINEISEMPTY" | "ISACTIVE"
        ) {
            let value = match name.as_str() {
                "ISSKIP" => self.skip_print,
                "MESSKIP" | "MOUSESKIP" => self.message_skip,
                "LINEISEMPTY" => self.presentation.last_line_is_empty(),
                "ISACTIVE" => self.client_focused,
                _ => unreachable!(),
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(value))),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SETANIMETIMER" {
            let milliseconds = integer_argument_value(&request.arguments, 0)?;
            let result = self
                .project_snapshot
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("SETANIMETIMER has no project".into()))?
                .resource_graph
                .set_animation_timer(milliseconds);
            if let Err(message) = result {
                return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
            }
            self.sync_resource_replay();
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_presentation();
        }
        if self.skip_print && is_runtime_print_command(&name) {
            if self.user_defined_skip && is_input_command(&name) {
                return self.fault(
                    FaultCode::VmFault,
                    "an input command cannot execute while user SKIPDISP is active; wrap it in NOSKIP/ENDNOSKIP",
                    Some(request.origin.clone()),
                );
            }
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "AWAIT" {
            let milliseconds = match request.arguments.first() {
                None | Some(VmValue::Integer(0)) => 0,
                Some(VmValue::Integer(value @ 1..=10_000)) => *value,
                _ => {
                    return self.fault(
                        FaultCode::VmFault,
                        "AWAIT duration must be between 0 and 10000 milliseconds",
                        Some(request.origin.clone()),
                    );
                }
            };
            if milliseconds == 0 {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability: HostWaitStability::Transient,
                    rebind_payload: Vec::new(),
                },
            )?;
            self.operations.insert_delay(
                request.id,
                self.logical_time_ns
                    .saturating_add(milliseconds.cast_unsigned().saturating_mul(1_000_000)),
            );
            return Ok(());
        }
        if matches!(
            name.as_str(),
            "QUIT" | "FORCE_QUIT" | "QUIT_AND_RESTART" | "FORCE_QUIT_AND_RESTART"
        ) {
            let exit = ExitRequested {
                reason: if name.ends_with("AND_RESTART") {
                    ExitReason::Restart
                } else {
                    ExitReason::Quit
                },
                force: name.starts_with("FORCE_"),
                runtime_revision: self.revision.saturating_add(1),
            };
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.exit_requested = Some(exit);
            self.emit(RuntimeMessage::ExitRequested(exit), None)?;
            return self.set_phase(RuntimePhase::Stopping);
        }
        if name == "CHKFONT" {
            let font = string_argument_value(&request.arguments, 0, "CHKFONT")?;
            let available = self.available_fonts.contains(&font.to_lowercase());
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(available))),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(name.as_str(), "GETCONFIG" | "GETCONFIGS") {
            let key = string_argument_value(&request.arguments, 0, &name)?;
            let project = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("GETCONFIG has no loaded project".into()))?;
            let replace = &vm.vm().artifact().project_data.static_data.replace;
            let value = if name == "GETCONFIG" {
                let value = match key {
                    "オートセーブを行なう" | "Make autosaves" => {
                        i64::from(project.auto_save)
                    }
                    "単位の位置" | "Currency symbol position" => {
                        i64::from(project.money_first)
                    }
                    "ウィンドウ幅" | "Window width" => i64::from(project.viewport_width),
                    "PRINTCを並べる数" | "Items per line for PRINTC" => {
                        i64::from(project.print_c_per_line)
                    }
                    "PRINTCの文字数" | "Number of Item characters for PRINTC" => {
                        i64::from(project.print_c_length)
                    }
                    "フォントサイズ" | "Font size" => i64::from(project.font_size),
                    "一行の高さ" | "Line height" => i64::from(project.line_height),
                    "表示するセーブデータ数" | "Save data count per page" => {
                        i64::from(project.save_slot_count)
                    }
                    "販売アイテム数" | "Max shop item storage" => {
                        i64::from(project.maximum_shop_items)
                    }
                    "COM_ABLE初期値" | "COM_ABLE initial value" => {
                        i64::from(replace.com_able_default)
                    }
                    "PBANDの初期値" | "PBAND initial value" => replace.pband_default,
                    "RELATIONの初期値" | "RELATION initial value" => replace.relation_default,
                    _ => {
                        return self.fault(
                            FaultCode::VmFault,
                            &format!("GETCONFIG does not expose configuration key {key:?}"),
                            Some(request.origin.clone()),
                        );
                    }
                };
                VmValue::Integer(value)
            } else {
                let value = match key {
                    "お金の単位" | "Currency symbol" => project.money_label.clone(),
                    "起動時簡略表示" | "Loading message" => replace.load_label.clone(),
                    "DRAWLINE文字" | "DRAWLINE characters" => replace.draw_line_string.clone(),
                    "システムメニュー0" | "System menu 0" => {
                        replace.title_menu_string_0.clone()
                    }
                    "システムメニュー1" | "System menu 1" => {
                        replace.title_menu_string_1.clone()
                    }
                    "時間切れ表示" | "Time-up message" => replace.timeup_label.clone(),
                    "BAR文字1" | "BAR character 1" => replace.bar_char_1.to_string(),
                    "BAR文字2" | "BAR character 2" => replace.bar_char_2.to_string(),
                    _ => {
                        return self.fault(
                            FaultCode::VmFault,
                            &format!("GETCONFIGS does not expose configuration key {key:?}"),
                            Some(request.origin.clone()),
                        );
                    }
                };
                VmValue::String(value)
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(value),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "VARSIZE" {
            let variable = string_argument_value(&request.arguments, 0, "VARSIZE")?;
            let dimensions = vm
                .variable_dimensions(request.fiber, variable)
                .ok_or_else(|| {
                    RuntimeError::Internal(format!(
                        "VARSIZE argument is not a variable: {variable}"
                    ))
                })?;
            let dimension = request
                .arguments
                .get(1)
                .map(|_| integer_argument_value(&request.arguments, 1))
                .transpose()?
                .unwrap_or(0);
            let dimension = usize::try_from(dimension).map_err(|_| {
                RuntimeError::Internal("VARSIZE dimension must be non-negative".into())
            })?;
            let value = dimensions.get(dimension).copied().ok_or_else(|| {
                RuntimeError::Internal("VARSIZE dimension exceeds the variable rank".into())
            })?;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::try_from(value).unwrap_or(i64::MAX))),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "EXISTFUNCTION" {
            let function = string_argument_value(&request.arguments, 0, "EXISTFUNCTION")?;
            let insensitive = request
                .arguments
                .get(1)
                .map(|_| integer_argument_value(&request.arguments, 1))
                .transpose()?
                .unwrap_or(0)
                != 0;
            let found = vm.vm().artifact().functions.iter().find(|candidate| {
                if insensitive {
                    candidate.name.eq_ignore_ascii_case(function)
                } else {
                    candidate.name == function
                }
            });
            let value = found.map_or(0, |function| match function.result {
                Some(erabasic_bytecode::BytecodeType::Integer) => 2,
                Some(erabasic_bytecode::BytecodeType::String) => 3,
                _ => 1,
            });
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "EXISTVAR" {
            let variable = string_argument_value(&request.arguments, 0, "EXISTVAR")?;
            let value = vm
                .vm()
                .artifact()
                .globals
                .iter()
                .find(|definition| {
                    definition.owner.is_none() && definition.name.eq_ignore_ascii_case(variable)
                })
                .map_or(0, |definition| {
                    let mut flags = match definition.value_type {
                        erabasic_bytecode::BytecodeType::Integer
                        | erabasic_bytecode::BytecodeType::IntegerPlace => 1,
                        erabasic_bytecode::BytecodeType::String
                        | erabasic_bytecode::BytecodeType::StringPlace => 2,
                    };
                    if !definition.mutable {
                        flags |= 4;
                    }
                    if definition.dimensions.len() == 2 {
                        flags |= 8;
                    } else if definition.dimensions.len() == 3 {
                        flags |= 16;
                    }
                    flags
                });
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "GETDOINGFUNCTION" {
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(request.origin.function_name.clone())),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "ENUMFUNCBEGINSWITH"
                | "ENUMFUNCENDSWITH"
                | "ENUMFUNCWITH"
                | "ENUMVARBEGINSWITH"
                | "ENUMVARENDSWITH"
                | "ENUMVARWITH"
        ) {
            let query = string_argument_value(&request.arguments, 0, &name)?;
            let target = request.arguments.get(1).and_then(|value| match value {
                VmValue::StringPlace(place) => Some(place.clone()),
                _ => None,
            });
            let mut names = Vec::new();
            if !query.is_empty() {
                let event_functions: BTreeSet<_> = vm
                    .vm()
                    .artifact()
                    .event_groups
                    .iter()
                    .flat_map(|group| {
                        group
                            .only
                            .iter()
                            .chain(&group.priority)
                            .chain(&group.normal)
                            .chain(&group.later)
                    })
                    .map(|entry| entry.function)
                    .collect();
                let candidates: Vec<&str> = if name.starts_with("ENUMFUNC") {
                    vm.vm()
                        .artifact()
                        .functions
                        .iter()
                        .filter(|function| !event_functions.contains(&function.key))
                        .map(|function| function.name.as_str())
                        .collect()
                } else {
                    let mut seen = BTreeSet::new();
                    vm.vm()
                        .artifact()
                        .globals
                        .iter()
                        .filter(|variable| {
                            variable.owner.is_none()
                                && seen.insert(variable.name.to_ascii_uppercase())
                        })
                        .map(|variable| variable.name.as_str())
                        .collect()
                };
                names.extend(
                    candidates
                        .into_iter()
                        .filter(|candidate| enum_name_matches(&name, candidate, query))
                        .map(str::to_owned),
                );
            }
            let writes = string_array_writes(vm, target, &names);
            let output_length = i64::try_from(writes.len()).unwrap_or(i64::MAX);
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(output_length)),
                    writes,
                }),
            );
        }
        if name == "BARSTR" {
            let value = integer_argument_value(&request.arguments, 0)?;
            let maximum = integer_argument_value(&request.arguments, 1)?;
            let length = integer_argument_value(&request.arguments, 2)?;
            if maximum <= 0 {
                return self.fault(
                    FaultCode::VmFault,
                    "BARSTR maximum must be positive",
                    Some(request.origin.clone()),
                );
            }
            if !(1..100).contains(&length) {
                return self.fault(
                    FaultCode::VmFault,
                    "BARSTR length must be between 1 and 99",
                    Some(request.origin.clone()),
                );
            }
            let replace = &vm.vm().artifact().project_data.static_data.replace;
            // Emuera performs the multiplication in an unchecked Int64 context.
            let filled = value.wrapping_mul(length) / maximum;
            let filled = filled.clamp(0, length);
            let empty = length - filled;
            let mut bar = String::from("[");
            bar.push_str(
                &replace
                    .bar_char_1
                    .to_string()
                    .repeat(usize::try_from(filled).unwrap_or(0)),
            );
            bar.push_str(
                &replace
                    .bar_char_2
                    .to_string()
                    .repeat(usize::try_from(empty).unwrap_or(0)),
            );
            bar.push(']');
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(bar)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(name.as_str(), "MONEYSTR" | "TOSTR") {
            let value = integer_argument_value(&request.arguments, 0)?;
            let formatted = match request.arguments.get(1) {
                None => value.to_string(),
                Some(VmValue::String(format)) => match format_era_integer(value, format) {
                    Ok(value) => value,
                    Err(error) => {
                        return self.fault(
                            FaultCode::VmFault,
                            &format!("{name} format is invalid: {error}"),
                            Some(request.origin.clone()),
                        );
                    }
                },
                Some(_) => {
                    return self.fault(
                        FaultCode::VmFault,
                        &format!("{name} argument 2 must be a string"),
                        Some(request.origin.clone()),
                    );
                }
            };
            if name == "TOSTR" {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(VmValue::String(formatted)),
                        writes: Vec::new(),
                    }),
                );
            }
            let project = self
                .project_snapshot
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("MONEYSTR has no loaded project".into()))?;
            let value = if project.money_first {
                format!("{}{formatted}", project.money_label)
            } else {
                format!("{formatted}{}", project.money_label)
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(name.as_str(), "TOFULL" | "TOHALF") {
            let value = string_argument_value(&request.arguments, 0, &name)?;
            let converted = if name == "TOFULL" {
                to_full_width(value)
            } else {
                to_half_width(value)
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(converted)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "CALLTRAIN" {
            let count =
                usize::try_from(integer_argument_value(&request.arguments, 0)?).map_err(|_| {
                    RuntimeError::Internal("CALLTRAIN count must be non-negative".into())
                })?;
            self.controller.continuous_commands.clear();
            for index in 0..count {
                self.controller
                    .continuous_commands
                    .push_back(read_runtime_integer(
                        vm,
                        "SELECTCOM",
                        &[u64::try_from(index + 1).unwrap_or(u64::MAX)],
                        None,
                    )?);
            }
            self.controller.continuous_train = true;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "STOPCALLTRAIN" {
            self.controller.continuous_commands.clear();
            self.controller.continuous_train = false;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "DOTRAIN" {
            let command = integer_argument_value(&request.arguments, 0)?;
            if command < 0 || self.controller.flow != Some(SystemFlow::Train) {
                return self.fault(
                    FaultCode::VmFault,
                    "DOTRAIN is only valid with a non-negative command during TRAIN",
                    Some(request.origin.clone()),
                );
            }
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.clear();
            self.controller.continuous_commands.clear();
            self.controller.continuous_train = false;
            self.controller.selected_command = Some(command);
            write_runtime_integer(vm, "SELECTCOM", &[], None, command)?;
            fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
            self.controller.step = SystemStep::TrainEventCom;
            if !self.dispatch_system_event(vm, "EVENTCOM")? {
                return self.continue_system_flow(vm);
            }
            return Ok(());
        }
        if matches!(name.as_str(), "BEGIN" | "FORCE_BEGIN") {
            let Some(VmValue::String(keyword)) = request.arguments.first() else {
                return self.fault(
                    FaultCode::VmFault,
                    "BEGIN expects a system keyword",
                    Some(request.origin.clone()),
                );
            };
            let Some(flow) = SystemFlow::parse(keyword) else {
                return self.fault(
                    FaultCode::VmFault,
                    &format!("unknown BEGIN system keyword: {keyword}"),
                    Some(request.origin.clone()),
                );
            };
            // The pinned fork treats BEGIN and FORCE_BEGIN as the same forced
            // transition. The current fiber cannot resume after changing systems.
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.clear();
            self.controller.flow = Some(flow);
            return self.begin_flow(vm, flow);
        }
        if matches!(name.as_str(), "SAVEVAR" | "LOADVAR") {
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("{name} is not implemented by the pinned reference runtime"),
                Some(request.origin.clone()),
            );
        }
        if name == "PUTFORM" {
            let suffix = request
                .arguments
                .first()
                .map(display_value)
                .unwrap_or_default();
            let variable = runtime_variable_key(vm, "SAVEDATA_TEXT")?;
            let current = vm
                .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
                    variable,
                    indices: Vec::new(),
                    character: None,
                }])
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let [VmValue::String(value)] = current.as_slice() else {
                return Err(RuntimeError::Internal(
                    "SAVEDATA_TEXT is not a scalar string".into(),
                ));
            };
            let mut value = value.clone();
            value.push_str(&suffix);
            let prepared = vm
                .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
                    writes: vec![VmRuntimeWrite {
                        variable,
                        indices: Vec::new(),
                        character: None,
                        value: VmValue::String(value),
                    }],
                    fills: Vec::new(),
                    clear_characters: false,
                    add_characters_from_csv: Vec::new(),
                })
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            vm.commit_runtime_state(prepared)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SAVENOS" {
            let count = self
                .project_snapshot
                .as_ref()
                .map_or(20, |snapshot| snapshot.save_slot_count);
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(count))),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(name.as_str(), "SAVEGAME" | "LOADGAME") {
            if !matches!(
                self.controller.flow,
                Some(SystemFlow::Shop | SystemFlow::Normal)
            ) {
                return self.fault(
                    FaultCode::VmFault,
                    &format!("{name} cannot open outside the reference __CAN_SAVE__ states"),
                    Some(request.origin.clone()),
                );
            }
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability: HostWaitStability::StableInput,
                    rebind_payload: name.as_bytes().to_vec(),
                },
            )?;
            self.system_menu_host_request = Some(request.id);
            let save = name == "SAVEGAME";
            self.system_menu = if save {
                SystemMenuState::SaveSlots
            } else {
                SystemMenuState::LoadSlots
            };
            return self.issue_storage(
                if save {
                    PendingStorage::ListSaveSlots
                } else {
                    PendingStorage::ListLoadSlots
                },
                StorageNamespace::Save,
                StorageOperation::List {
                    pattern: Some("save*.sav".into()),
                    recursive: false,
                },
                String::new(),
            );
        }
        if matches!(name.as_str(), "RESETDATA" | "RESETGLOBAL") {
            let transaction = if name == "RESETDATA" {
                VmRuntimeStateTransaction::ResetGameData
            } else {
                VmRuntimeStateTransaction::ResetGlobalData
            };
            let prepared = vm
                .prepare_runtime_state(transaction)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            vm.commit_runtime_state(prepared)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SAVEDATA" {
            let slot = save_slot_argument(&request.arguments, 0, "SAVEDATA")?;
            let description = string_argument_value(&request.arguments, 1, "SAVEDATA")?;
            if description.contains(['\r', '\n']) {
                return self.fault(
                    FaultCode::VmFault,
                    "SAVEDATA description cannot contain a newline",
                    Some(request.origin.clone()),
                );
            }
            let bytes = encode_scoped_save(
                &vm.export_era_state(),
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Normal,
                description.to_owned(),
                merge_structured_extensions(
                    &self.save_extensions,
                    vm.structured_extensions(StructuredScope::Ordinary)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                self.traditional_save_format(),
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostWrite {
                    request: request.id,
                },
                StorageNamespace::Save,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                save_slot_path(slot),
            );
        }
        if name == "LOADDATA" {
            let slot = save_slot_argument(&request.arguments, 0, "LOADDATA")?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadOrdinary { slot },
                StorageNamespace::Save,
                StorageOperation::Read,
                save_slot_path(slot),
            );
        }
        if name == "DELDATA" {
            let slot = save_slot_argument(&request.arguments, 0, "DELDATA")?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostDelete {
                    request: request.id,
                },
                StorageNamespace::Save,
                StorageOperation::Delete {
                    precondition: StoragePrecondition::Any,
                },
                save_slot_path(slot),
            );
        }
        if name == "SAVEGLOBAL" {
            let state = vm.vm().export_era_state_for(EraSaveScope::Global);
            let bytes = encode_scoped_save(
                &state,
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Global,
                String::new(),
                merge_structured_extensions(
                    &self.save_extensions,
                    vm.structured_extensions(StructuredScope::Global)
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                self.traditional_save_format(),
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostWrite {
                    request: request.id,
                },
                StorageNamespace::GlobalSave,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                "global.sav".into(),
            );
        }
        if name == "LOADGLOBAL" {
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadGlobal {
                    request: request.id,
                },
                StorageNamespace::GlobalSave,
                StorageOperation::Read,
                "global.sav".into(),
            );
        }
        if name == "SAVECHARA" {
            let filename =
                dat_filename(string_argument_value(&request.arguments, 0, "SAVECHARA")?)?;
            let description = string_argument_value(&request.arguments, 1, "SAVECHARA")?;
            let exported = vm.vm().export_era_state_for(EraSaveScope::Characters);
            let mut selected = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            for index in 2..request.arguments.len() {
                let value = usize::try_from(integer_argument_value(&request.arguments, index)?)
                    .map_err(|_| {
                        RuntimeError::Internal(format!(
                            "SAVECHARA argument {} must be non-negative",
                            index + 1
                        ))
                    })?;
                if value >= exported.characters.len() {
                    return Err(RuntimeError::Internal(format!(
                        "SAVECHARA argument {} is not a character",
                        index + 1
                    )));
                }
                if !seen.insert(value) {
                    return Err(RuntimeError::Internal(format!(
                        "SAVECHARA character {value} is duplicated"
                    )));
                }
                selected.push(exported.characters[value].clone());
            }
            let state = EraState {
                unique_code: exported.unique_code,
                version: exported.version,
                variables: BTreeMap::new(),
                characters: selected,
            };
            let bytes = encode_scoped_save(
                &state,
                vm.vm().artifact(),
                era_runtime_save::SaveFileKind::Character,
                description.to_owned(),
                Vec::new(),
                era_runtime_save::SaveFormat::Binary1808,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostWrite {
                    request: request.id,
                },
                StorageNamespace::Data,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                format!("chara_{filename}.dat"),
            );
        }
        if name == "LOADCHARA" {
            let filename =
                dat_filename(string_argument_value(&request.arguments, 0, "LOADCHARA")?)?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostLoadCharacters {
                    request: request.id,
                },
                StorageNamespace::Data,
                StorageOperation::Read,
                format!("chara_{filename}.dat"),
            );
        }
        if name == "CHKDATA" {
            let slot = save_slot_argument(&request.arguments, 0, "CHKDATA")?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostCheck {
                    request: request.id,
                    kind: era_runtime_save::SaveFileKind::Normal,
                },
                StorageNamespace::Save,
                StorageOperation::Read,
                save_slot_path(slot),
            );
        }
        if name == "CHKCHARADATA" {
            let filename = dat_filename(string_argument_value(
                &request.arguments,
                0,
                "CHKCHARADATA",
            )?)?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostCheck {
                    request: request.id,
                    kind: era_runtime_save::SaveFileKind::Character,
                },
                StorageNamespace::Data,
                StorageOperation::Read,
                format!("chara_{filename}.dat"),
            );
        }
        if name == "SAVETEXT" {
            let text = string_argument_value(&request.arguments, 0, "SAVETEXT")?;
            let (namespace, path) = text_storage_target(
                request
                    .arguments
                    .get(1)
                    .ok_or_else(|| RuntimeError::Internal("SAVETEXT target is missing".into()))?,
            )?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostFunctionWrite {
                    request: request.id,
                },
                namespace,
                StorageOperation::Write {
                    data: ProtocolBytes::new(text.as_bytes().to_vec()),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                path,
            );
        }
        if name == "LOADTEXT" {
            let (namespace, path) = text_storage_target(
                request
                    .arguments
                    .first()
                    .ok_or_else(|| RuntimeError::Internal("LOADTEXT target is missing".into()))?,
            )?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostReadText {
                    request: request.id,
                },
                namespace,
                StorageOperation::Read,
                path,
            );
        }
        if name == "EXISTFILE" {
            let path =
                safe_relative_path(string_argument_value(&request.arguments, 0, "EXISTFILE")?)?;
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostStat {
                    request: request.id,
                },
                StorageNamespace::Data,
                StorageOperation::Stat,
                path,
            );
        }
        if name == "ENUMFILES" {
            let directory = safe_relative_directory(string_argument_value(
                &request.arguments,
                0,
                "ENUMFILES",
            )?)?;
            let pattern = request.arguments.get(1).and_then(|value| match value {
                VmValue::String(value) => Some(value.clone()),
                _ => None,
            });
            let recursive =
                matches!(request.arguments.get(2), Some(VmValue::Integer(value)) if *value != 0);
            let target = request.arguments.get(3).and_then(|value| match value {
                VmValue::StringPlace(place) => Some(place.clone()),
                _ => None,
            });
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostListFiles {
                    request: request.id,
                    target,
                    strip_character_dat: false,
                },
                StorageNamespace::Data,
                StorageOperation::List { pattern, recursive },
                directory,
            );
        }
        if name == "FIND_CHARADATA" {
            let pattern = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::String(value) => Some(value.as_str()),
                    _ => None,
                })
                .unwrap_or("*");
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostListFiles {
                    request: request.id,
                    target: None,
                    strip_character_dat: true,
                },
                StorageNamespace::Data,
                StorageOperation::List {
                    pattern: Some(format!("chara_{pattern}.dat")),
                    recursive: false,
                },
                String::new(),
            );
        }
        if name == "OUTPUTLOG" {
            let filename = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::String(value) if !value.is_empty() => Some(value.as_str()),
                    _ => None,
                })
                .unwrap_or("emuera.log");
            let path = safe_relative_path(filename)?;
            let hide_info = matches!(request.arguments.get(1), Some(VmValue::Integer(1)));
            let mut data = vec![0xef, 0xbb, 0xbf];
            data.extend_from_slice(self.presentation.log_text(hide_info).as_bytes());
            return self.issue_host_storage(
                vm,
                request,
                PendingStorage::HostFunctionWrite {
                    request: request.id,
                },
                StorageNamespace::Log,
                StorageOperation::Write {
                    data: ProtocolBytes::new(data),
                    atomic_replace: true,
                    precondition: StoragePrecondition::Any,
                },
                path,
            );
        }
        if let Some(mut pending) = input_wait(
            request,
            self.allocate_wait(),
            self.allocate_interaction(),
            self.logical_time_ns,
        ) {
            if matches!(
                pending.wait.kind,
                WaitKind::IntegerValue | WaitKind::IntegerButton
            ) {
                pending.choices = std::mem::take(&mut self.command_intents);
            }
            if pending.wait.stop_message_skip {
                self.message_skip = false;
            }
            if self.message_skip
                && matches!(
                    name.as_str(),
                    "TINPUT" | "TONEINPUT" | "TINPUTS" | "TONEINPUTS"
                )
                && request.arguments.len() >= 6
            {
                let mouse = matches!(request.arguments.get(4), Some(VmValue::Integer(1)));
                let target = pending.result_name.as_deref().and_then(|result| {
                    if mouse {
                        global_place_at(vm, result, 1)
                    } else {
                        global_place(vm, result)
                    }
                });
                let value = pending
                    .wait
                    .default_value
                    .as_ref()
                    .map_or(VmValue::Integer(0), protocol_to_vm);
                let writes = target
                    .map(|target| vec![HostWrite { target, value }])
                    .unwrap_or_default();
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: None,
                        writes,
                    }),
                );
            }
            let stability = match pending.wait.stability {
                WaitStability::StableInput => HostWaitStability::StableInput,
                WaitStability::Transient => HostWaitStability::Transient,
            };
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability,
                    rebind_payload: encode_canonical(&pending.wait)?,
                },
            )?;
            return self.open_wait(pending, false);
        }
        if name == "GETLINESTR" {
            let Some(VmValue::String(pattern)) = request.arguments.first() else {
                return self.fault(
                    FaultCode::VmFault,
                    "GETLINESTR expects a string pattern",
                    Some(request.origin.clone()),
                );
            };
            let value = match logical_line_string(pattern, 75) {
                Ok(value) => value,
                Err(message) => {
                    return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
                }
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "CLIENTWIDTH" | "CLIENTHEIGHT" | "PRINTCPERLINE" | "PRINTCLENGTH"
        ) {
            let project = self.project_snapshot.as_ref().ok_or_else(|| {
                RuntimeError::Internal("layout query has no loaded project".into())
            })?;
            let value = match name.as_str() {
                "CLIENTWIDTH" => project.viewport_width,
                "CLIENTHEIGHT" => project.viewport_height,
                "PRINTCPERLINE" => project.print_c_per_line,
                _ => project.print_c_length,
            };
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(value))),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "GETDISPLAYLINE"
                | "HTML_GETPRINTEDSTR"
                | "HTML_POPPRINTINGSTR"
                | "HTML_STRINGLEN"
                | "HTML_SUBSTRING"
                | "HTML_STRINGLINES"
        ) {
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("unsupported runtime presentation query: {name}"),
                Some(request.origin.clone()),
            );
        }
        if name == "DRAWLINE" {
            let pattern = request
                .arguments
                .first()
                .map_or_else(|| "-".into(), display_value);
            self.presentation.append_separator(pattern);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "CLEARLINE" {
            let count = request
                .arguments
                .first()
                .and_then(|value| match value {
                    VmValue::Integer(value) => usize::try_from(*value).ok(),
                    _ => None,
                })
                .unwrap_or(1);
            self.presentation.delete_last_lines(count);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "HTML_PRINT" {
            self.presentation.append_html(
                request
                    .arguments
                    .first()
                    .map_or_else(String::new, display_value),
            );
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "ALIGNMENT" {
            let alignment = match request.arguments.first() {
                Some(VmValue::Integer(1)) => LineAlignment::Center,
                Some(VmValue::Integer(2)) => LineAlignment::Right,
                _ => LineAlignment::Left,
            };
            self.presentation.set_alignment(alignment);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "FONTSTYLE" {
            let bits = integer_argument_value(&request.arguments, 0)?;
            self.presentation.set_font_style(bits);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SETFONT" {
            let family = request.arguments.first().map(display_value);
            self.presentation.set_font(family);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SETCOLOR" {
            self.presentation
                .set_foreground(integer_argument_value(&request.arguments, 0)?);
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "SETBGCOLOR" {
            self.presentation
                .set_background(integer_argument_value(&request.arguments, 0)?);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "SETBGIMAGE" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let depth = request.arguments.get(1).map_or(0, integer_value_or_zero);
            let opacity = request.arguments.get(2).map_or(255, integer_value_or_zero);
            let exists = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite(&resource))
                .is_some();
            if exists {
                self.presentation.add_background(resource, depth, opacity);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "REMOVEBGIMAGE" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            self.presentation.remove_background(&resource);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "CLEARBGIMAGE" {
            self.presentation.clear_backgrounds();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name.starts_with("TOOLTIP_") {
            let result = match name.as_str() {
                "TOOLTIP_SETCOLOR" => {
                    let foreground = integer_argument_value(&request.arguments, 0)?;
                    let background = integer_argument_value(&request.arguments, 1)?;
                    if !(0..=0xff_ffff).contains(&foreground)
                        || !(0..=0xff_ffff).contains(&background)
                    {
                        Err("tooltip color is out of range")
                    } else {
                        self.presentation.set_tooltip_colors(foreground, background);
                        Ok(())
                    }
                }
                "TOOLTIP_SETDELAY" => self
                    .presentation
                    .set_tooltip_delay(integer_argument_value(&request.arguments, 0)?),
                "TOOLTIP_SETDURATION" => self
                    .presentation
                    .set_tooltip_duration(integer_argument_value(&request.arguments, 0)?),
                "TOOLTIP_SETFONT" => {
                    self.presentation.set_tooltip_font(
                        request
                            .arguments
                            .first()
                            .map_or_else(String::new, display_value),
                    );
                    Ok(())
                }
                "TOOLTIP_SETFONTSIZE" => self
                    .presentation
                    .set_tooltip_font_size(integer_argument_value(&request.arguments, 0)?),
                "TOOLTIP_CUSTOM" => {
                    self.presentation
                        .set_tooltip_custom(integer_argument_value(&request.arguments, 0)? != 0);
                    Ok(())
                }
                "TOOLTIP_FORMAT" => {
                    self.presentation
                        .set_tooltip_format(integer_argument_value(&request.arguments, 0)?);
                    Ok(())
                }
                "TOOLTIP_IMG" => {
                    self.presentation
                        .set_tooltip_images(integer_argument_value(&request.arguments, 0)? != 0);
                    Ok(())
                }
                _ => Err("unsupported tooltip operation"),
            };
            if let Err(message) = result {
                return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_IMG" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let exists = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite(&resource))
                .is_some();
            if exists {
                self.presentation.append_image(resource, None);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "PRINT_RECT" {
            let parameters = request
                .arguments
                .iter()
                .filter_map(|value| match value {
                    VmValue::Integer(value) => Some(*value),
                    _ => None,
                })
                .collect();
            self.presentation.append_rectangle(parameters);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "EXISTSOUND" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let exists = self
                .project_snapshot
                .as_ref()
                .is_some_and(|project| project.resource_graph.contains_audio(&resource));
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(i64::from(exists))),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name.as_str(),
            "SPRITECREATED" | "SPRITEWIDTH" | "SPRITEHEIGHT" | "SPRITEPOSX" | "SPRITEPOSY"
        ) {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let value = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite(&resource))
                .map_or(0, |sprite| match name.as_str() {
                    "SPRITECREATED" => 1,
                    "SPRITEWIDTH" => i64::from(sprite.width),
                    "SPRITEHEIGHT" => i64::from(sprite.height),
                    "SPRITEPOSX" => i64::from(sprite.position_x),
                    _ => i64::from(sprite.position_y),
                });
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::Integer(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SPRITEGETCOLOR" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let x = integer_argument_value(&request.arguments, 1)?;
            let y = integer_argument_value(&request.arguments, 2)?;
            let Some((resource_id, digest, x, y)) = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite_pixel_request(&resource, x, y))
            else {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady {
                        value: Some(VmValue::Integer(-1)),
                        writes: Vec::new(),
                    }),
                );
            };
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::SpritePixel {
                    request: request.id,
                },
                ServiceKind::Image,
                IMAGE_PIXEL_OPERATION,
                IMAGE_PIXEL_OPERATION_VERSION,
                &ImagePixelRequest {
                    resource_id,
                    content_digest: ProtocolBytes::new(digest),
                    x,
                    y,
                },
            );
        }
        if matches!(name.as_str(), "SPRITEMOVE" | "SPRITESETPOS") {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let x = i32::try_from(integer_argument_value(&request.arguments, 1)?).unwrap_or(0);
            let y = i32::try_from(integer_argument_value(&request.arguments, 2)?).unwrap_or(0);
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .move_sprite(&resource, x, y, name == "SPRITEMOVE")
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GCREATE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let width = integer_argument_value(&request.arguments, 1)?;
            let height = integer_argument_value(&request.arguments, 2)?;
            let result = self
                .project_snapshot
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("GCREATE has no loaded project".into()))?
                .resource_graph
                .create_canvas(id, width, height);
            let created = match result {
                Ok(value) => value,
                Err(message) => {
                    return self.fault(FaultCode::VmFault, message, Some(request.origin.clone()));
                }
            };
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "GDISPOSE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_canvas(id));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        if matches!(name.as_str(), "GCREATED" | "GWIDTH" | "GHEIGHT") {
            let id = integer_argument_value(&request.arguments, 0)?;
            let state = self
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.canvas_state(id));
            let value = match (name.as_str(), state) {
                ("GCREATED", Some(_)) => 1,
                ("GWIDTH", Some((width, _))) => i64::from(width),
                ("GHEIGHT", Some((_, height))) => i64::from(height),
                _ => 0,
            };
            return commit_integer_result(vm, request.id, value);
        }
        if name == "GCLEAR" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let color = integer_argument_value(&request.arguments, 1)?;
            let rectangle = if request.arguments.len() == 6 {
                Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    i32_argument_value(&request.arguments, 4)?,
                    i32_argument_value(&request.arguments, 5)?,
                ])
            } else {
                None
            };
            let changed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.clear_canvas(id, color, rectangle));
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "GDRAWSPRITE" {
            let id = integer_argument_value(&request.arguments, 0)?;
            let sprite = request
                .arguments
                .get(1)
                .map_or_else(String::new, display_value);
            let destination = match request.arguments.len() {
                2 => None,
                4 => Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    self.project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.sprite(&sprite))
                        .map_or(0, |value| i32::try_from(value.width).unwrap_or(i32::MAX)),
                    self.project_snapshot
                        .as_ref()
                        .and_then(|project| project.resource_graph.sprite(&sprite))
                        .map_or(0, |value| i32::try_from(value.height).unwrap_or(i32::MAX)),
                ]),
                _ => Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    i32_argument_value(&request.arguments, 4)?,
                    i32_argument_value(&request.arguments, 5)?,
                ]),
            };
            let changed = self.project_snapshot.as_mut().is_some_and(|project| {
                project.resource_graph.draw_sprite(id, &sprite, destination)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(changed));
        }
        if name == "SPRITEANIMECREATE" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let width = integer_argument_value(&request.arguments, 1)?;
            let height = integer_argument_value(&request.arguments, 2)?;
            if !(1..=8_192).contains(&width) || !(1..=8_192).contains(&height) {
                return self.fault(
                    FaultCode::VmFault,
                    "animation sprite dimensions are out of range",
                    Some(request.origin.clone()),
                );
            }
            let created = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .create_animation_sprite(&sprite, width, height)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "SPRITEANIMEADDFRAME" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let canvas_id = integer_argument_value(&request.arguments, 1)?;
            let rectangle = [
                i32_argument_value(&request.arguments, 2)?,
                i32_argument_value(&request.arguments, 3)?,
                i32_argument_value(&request.arguments, 4)?,
                i32_argument_value(&request.arguments, 5)?,
            ];
            let offset = [
                i32_argument_value(&request.arguments, 6)?,
                i32_argument_value(&request.arguments, 7)?,
            ];
            let delay = integer_argument_value(&request.arguments, 8)?;
            let added = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .add_animation_frame(&sprite, canvas_id, rectangle, offset, delay)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(added));
        }
        if name == "SPRITECREATE" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let id = integer_argument_value(&request.arguments, 1)?;
            let rectangle = if request.arguments.len() == 6 {
                Some([
                    i32_argument_value(&request.arguments, 2)?,
                    i32_argument_value(&request.arguments, 3)?,
                    i32_argument_value(&request.arguments, 4)?,
                    i32_argument_value(&request.arguments, 5)?,
                ])
            } else {
                None
            };
            let created = self.project_snapshot.as_mut().is_some_and(|project| {
                project
                    .resource_graph
                    .create_canvas_sprite(&sprite, id, rectangle)
            });
            return self.complete_graphics_result(vm, request.id, i64::from(created));
        }
        if name == "SPRITEDISPOSE" {
            let sprite = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let disposed = self
                .project_snapshot
                .as_mut()
                .is_some_and(|project| project.resource_graph.dispose_sprite(&sprite));
            return self.complete_graphics_result(vm, request.id, i64::from(disposed));
        }
        if name == "SPRITEDISPOSEALL" {
            let include_static = integer_argument_value(&request.arguments, 0)? != 0;
            let count = self.project_snapshot.as_mut().map_or(0, |project| {
                project.resource_graph.dispose_sprites(include_static)
            });
            return self.complete_graphics_result(
                vm,
                request.id,
                i64::try_from(count).unwrap_or(i64::MAX),
            );
        }
        if matches!(name.as_str(), "PLAYBGM" | "PLAYSOUND") {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let bgm = name == "PLAYBGM";
            let exists = self
                .project_snapshot
                .as_ref()
                .is_some_and(|project| project.resource_graph.contains_audio(&resource));
            if !exists {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            self.presentation.set_audio(resource.clone(), bgm, true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::Play,
                    resource_id: Some(resource),
                    repeat_count: if bgm { -1 } else { 1 },
                    volume_millionths: 1_000_000,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "STOPBGM" | "STOPSOUND") {
            let bgm = name == "STOPBGM";
            self.presentation.set_audio(String::new(), bgm, false);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::Stop,
                    resource_id: None,
                    repeat_count: 0,
                    volume_millionths: 0,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(name.as_str(), "SETBGMVOLUME" | "SETSOUNDVOLUME") {
            let bgm = name == "SETBGMVOLUME";
            let volume = integer_argument_value(&request.arguments, 0)?;
            self.presentation.set_audio_volume(bgm, volume);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            self.emit_presentation()?;
            if self.presentation.projects_audio() && self.client_audio_available {
                return self.emit_effect(EffectKind::Audio(AudioEffect {
                    channel_id: u64::from(bgm),
                    action: AudioEffectAction::SetVolume,
                    resource_id: None,
                    repeat_count: 0,
                    volume_millionths: u32::try_from(volume.clamp(0, 100)).unwrap_or_default()
                        * 10_000,
                }));
            }
            if self.presentation.projects_audio() {
                return self.emit_audio_unavailable();
            }
            return Ok(());
        }
        if matches!(
            name.as_str(),
            "PRINTBUTTON" | "PRINTBUTTONC" | "PRINTBUTTONLC"
        ) {
            let text = request
                .arguments
                .first()
                .map_or_else(String::new, display_value)
                .replace('\n', "");
            let value = request
                .arguments
                .get(1)
                .cloned()
                .ok_or_else(|| RuntimeError::Internal("PRINTBUTTON value is missing".into()))?;
            let token = self.allocate_interaction();
            let alignment = match name.as_str() {
                "PRINTBUTTONC" => Some(CellAlignment::Right),
                "PRINTBUTTONLC" => Some(CellAlignment::Left),
                _ => None,
            };
            self.presentation.append_button(text, token, alignment);
            self.command_intents.insert(token, value);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if is_print(&name) {
            let text = request
                .arguments
                .iter()
                .map(display_value)
                .collect::<String>();
            if name == "REUSELASTLINE" {
                self.presentation.print_temporary_line(text);
            } else if is_column_print(&name) {
                let alignment = if name.ends_with("LC") {
                    CellAlignment::Left
                } else {
                    CellAlignment::Right
                };
                self.presentation.append_column_cell(text, alignment);
            } else {
                self.presentation
                    .append_print_text(text, false, print_commits_line(&name));
            }
            if name.ends_with('W') {
                let wait = InputWait {
                    wait_id: self.allocate_wait(),
                    kind: WaitKind::EnterKey,
                    stability: WaitStability::StableInput,
                    one_input: false,
                    stop_message_skip: false,
                    system_input: false,
                    mouse_input: false,
                    default_value: None,
                    deadline_ns: None,
                    display_time: false,
                    timeout_message: None,
                    submission_token: self.allocate_interaction(),
                    countdown_remaining_ms: None,
                };
                let pending = PendingInput {
                    host_request: Some(request.id),
                    wait,
                    result_name: None,
                    choices: BTreeMap::new(),
                    timeout_duration_ns: None,
                    post_input: None,
                };
                commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Pending {
                        stability: HostWaitStability::StableInput,
                        rebind_payload: encode_canonical(&pending.wait)?,
                    },
                )?;
                return self.open_wait(pending, false);
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if name == "UPDATECHECK" {
            let game_base = &vm.vm().artifact().project_data.static_data.game_base;
            if game_base.update_url.is_empty() {
                return commit_host_result_write(vm, request.id, 3);
            }
            return self.issue_host_service(
                vm,
                request,
                ExternalCompletion::UpdateCheck {
                    request: request.id,
                },
                ServiceKind::Network,
                UPDATE_CHECK_OPERATION,
                UPDATE_CHECK_OPERATION_VERSION,
                &UpdateCheckRequest {
                    url: game_base.update_url.clone(),
                },
            );
        }
        if matches!(name.as_str(), "GETKEY" | "GETKEYTRIGGERED") {
            let key = match request.arguments.first() {
                Some(VmValue::Integer(value)) => match u8::try_from(*value) {
                    Ok(value) => value,
                    Err(_) => {
                        return commit_completion(
                            vm,
                            request.id,
                            VmHostCompletion::Ready(HostReady {
                                value: Some(VmValue::Integer(0)),
                                writes: Vec::new(),
                            }),
                        );
                    }
                },
                _ => {
                    return commit_completion(
                        vm,
                        request.id,
                        VmHostCompletion::Ready(HostReady {
                            value: Some(VmValue::Integer(0)),
                            writes: Vec::new(),
                        }),
                    );
                }
            };
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::GetKey {
                    request: request.id,
                    key_code: key,
                    triggered: name == "GETKEYTRIGGERED",
                },
                ServiceKind::InputState,
                GET_KEY_STATE_OPERATION,
                GET_KEY_STATE_OPERATION_VERSION,
                &GetKeyStateRequest { key_code: key },
            )
        } else if matches!(
            name.as_str(),
            "GETTIME" | "GETTIMES" | "GETMILLISECOND" | "GETSECOND"
        ) {
            let operation = match name.as_str() {
                "GETTIMES" => ClockOperation::Times,
                "GETMILLISECOND" => ClockOperation::Millisecond,
                "GETSECOND" => ClockOperation::Second,
                _ => ClockOperation::Time,
            };
            self.issue_host_service(
                vm,
                request,
                ExternalCompletion::LocalDateTime {
                    request: request.id,
                    operation,
                    result: request.import.import.result,
                },
                ServiceKind::Clock,
                LOCAL_DATE_TIME_OPERATION,
                LOCAL_DATE_TIME_OPERATION_VERSION,
                &LocalDateTimeRequest {},
            )
        } else {
            self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!("unsupported host import: {}", request.import.import.name),
                Some(request.origin.clone()),
            )
        }
    }

    // The typed operation tuple is deliberately explicit at this single protocol edge.
    #[allow(clippy::too_many_arguments)]
    fn issue_host_service<T: minicbor::Encode<()>>(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        completion: ExternalCompletion,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.fault(
                FaultCode::UnsupportedRuntimeFeature,
                &format!(
                    "frontend did not negotiate service {kind:?}/{operation} {operation_version:?}"
                ),
                Some(request.origin.clone()),
            );
        }
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        let request_id = self.allocate_request()?;
        self.operations
            .insert_service(request_id, PendingService::Host(completion));
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    fn issue_platform_effect<T: minicbor::Encode<()>>(
        &mut self,
        kind: ServiceKind,
        operation: &str,
        operation_version: ProtocolVersion,
        payload: &T,
    ) -> Result<(), RuntimeError> {
        if self.service_capabilities.get(&(kind, operation.to_owned())) != Some(&operation_version)
        {
            return self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code: "runtime.platform_capability_unavailable".into(),
                    severity: DiagnosticSeverity::Warning,
                    message: format!("frontend did not negotiate service {kind:?}/{operation}"),
                    source: None,
                }),
                None,
            );
        }
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::PlatformEffect {
                operation: operation.into(),
            },
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
                deadline_ns: None,
            }),
            None,
        )
    }

    fn issue_host_storage(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Pending {
                stability: HostWaitStability::Transient,
                rebind_payload: Vec::new(),
            },
        )?;
        self.issue_storage(pending, namespace, operation, relative_path)
    }

    fn begin_candidate_save(
        &mut self,
        vm: &mut RuntimeVm,
        slot: u32,
        continuation: CandidateSaveContinuation,
    ) -> Result<(), RuntimeError> {
        let capabilities = self.storage_capabilities;
        if !(capabilities.revisions
            && capabilities.atomic_replace
            && capabilities.missing_precondition)
        {
            return match continuation {
                CandidateSaveContinuation::Autosave => {
                    self.emit(
                        RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                            code: "runtime.candidate_save_failed".into(),
                            severity: DiagnosticSeverity::Warning,
                            message:
                                "frontend storage cannot provide revision-checked atomic writes"
                                    .into(),
                            source: None,
                        }),
                        None,
                    )?;
                    self.presentation.append_system_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::AutoSaveFailed),
                        SystemTextKey::AutoSaveFailed,
                        Vec::new(),
                        false,
                    );
                    self.controller.step = SystemStep::ShopShow;
                    self.dispatch_system_function(vm, "SHOW_SHOP", true)
                        .map(|_| ())
                }
                CandidateSaveContinuation::SystemMenu { .. } => self.finish_candidate_save_failure(
                    continuation,
                    "frontend storage cannot provide revision-checked atomic writes",
                ),
            };
        }
        self.issue_storage(
            PendingStorage::CandidateSaveStat { slot, continuation },
            StorageNamespace::Save,
            StorageOperation::Stat,
            save_slot_path(slot),
        )
    }

    fn begin_system_menu_candidate(&mut self, slot: u32) -> Result<(), RuntimeError> {
        let request = self.system_menu_host_request.ok_or_else(|| {
            RuntimeError::Internal("system save menu lost its VM continuation".into())
        })?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("system save menu has no VM".into()))?;
        let result = self.begin_candidate_save(
            &mut vm,
            slot,
            CandidateSaveContinuation::SystemMenu { request },
        );
        self.vm = Some(vm);
        result
    }

    fn issue_candidate_clock(
        &mut self,
        slot: u32,
        precondition: StoragePrecondition,
        continuation: CandidateSaveContinuation,
    ) -> Result<(), RuntimeError> {
        if self
            .service_capabilities
            .get(&(ServiceKind::Clock, LOCAL_DATE_TIME_OPERATION.to_owned()))
            != Some(&LOCAL_DATE_TIME_OPERATION_VERSION)
        {
            return self.finish_candidate_save_failure(
                continuation,
                "frontend did not negotiate the candidate-save clock service",
            );
        }
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::CandidateSaveClock {
                slot,
                precondition,
                continuation,
            },
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind: ServiceKind::Clock,
                operation: LOCAL_DATE_TIME_OPERATION.into(),
                operation_version: LOCAL_DATE_TIME_OPERATION_VERSION,
                payload: ProtocolBytes::new(encode_canonical(&LocalDateTimeRequest {})?),
                deadline_ns: None,
            }),
            None,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn prepare_candidate_save(
        &mut self,
        time: LocalDateTimeResponse,
    ) -> Result<(PendingCandidateCommit, Vec<u8>), RuntimeError> {
        let mut candidate = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("candidate save has no VM".into()))?
            .fork_isolated()
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        write_runtime_string(
            &mut candidate,
            "SAVEDATA_TEXT",
            format!(
                "{:04}/{:02}/{:02} {:02}:{:02}:{:02} ",
                time.year, time.month, time.day, time.hour, time.minute, time.second
            ),
        )?;

        let function = candidate
            .vm()
            .artifact()
            .functions
            .iter()
            .find(|function| function.name.eq_ignore_ascii_case("SAVEINFO"))
            .map(|function| function.key);

        let original_presentation = self.presentation.clone();
        let original_project = self.project_snapshot.clone();
        let original_flags = (
            self.message_skip,
            self.skip_print,
            self.user_defined_skip,
            self.saved_skip,
        );
        let original_phase = self.phase;
        let original_revision = self.revision;
        let original_outbound = std::mem::take(&mut self.outbound);
        let original_outbound_journal = std::mem::take(&mut self.outbound_journal);
        let original_effect_journal = std::mem::take(&mut self.effect_journal);
        let original_sequence = self.outbound_sequence;
        let original_message = self.next_message_id;
        let original_effect = self.next_effect_id;
        self.candidate_clock = Some(time);

        let execution = (|| -> Result<(), RuntimeError> {
            let Some(function) = function else {
                return Ok(());
            };
            let fiber = candidate
                .spawn_entry(function, Vec::new())
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let maximum = self.options.limits.maximum_drive_instructions.max(1);
            let mut executed = 0_u64;
            loop {
                let report = candidate.drive(
                    RunBudget {
                        maximum_instructions: maximum.saturating_sub(executed).max(1),
                        maximum_host_calls: self.options.limits.maximum_pending_requests,
                        fiber_quantum: RunBudget::default().fiber_quantum,
                    },
                    VmDriveMode::Normal,
                );
                executed = executed.saturating_add(report.instructions);
                let mut completed = false;
                for event in report.events {
                    match event {
                        VmPortEvent::HostCall(request) => {
                            self.handle_host_call(&mut candidate, &request)?;
                        }
                        VmPortEvent::FiberCompleted(id, _) if id == fiber => completed = true,
                        VmPortEvent::FiberFaulted(id, fault) if id == fiber => {
                            return Err(RuntimeError::Internal(format!(
                                "candidate SAVEINFO faulted: {}",
                                fault.message
                            )));
                        }
                        VmPortEvent::FiberYielded(_)
                        | VmPortEvent::DebugStopped(_)
                        | VmPortEvent::FiberCompleted(_, _)
                        | VmPortEvent::FiberFaulted(_, _) => {}
                    }
                }
                if completed {
                    return Ok(());
                }
                if executed >= maximum {
                    return Err(RuntimeError::ResourceLimit(
                        "candidate SAVEINFO exceeded its instruction budget",
                    ));
                }
                if !candidate.has_runnable_fibers() {
                    return Err(RuntimeError::Internal(
                        "candidate SAVEINFO attempted to suspend".into(),
                    ));
                }
            }
        })();

        self.candidate_clock = None;
        let candidate_presentation = self.presentation.clone();
        let candidate_project = self.project_snapshot.clone();
        let candidate_flags = (
            self.message_skip,
            self.skip_print,
            self.user_defined_skip,
            self.saved_skip,
        );
        let effects = self
            .effect_journal
            .values()
            .map(|event| event.kind.clone())
            .collect();
        self.presentation = original_presentation;
        self.project_snapshot = original_project;
        (
            self.message_skip,
            self.skip_print,
            self.user_defined_skip,
            self.saved_skip,
        ) = original_flags;
        self.phase = original_phase;
        self.revision = original_revision;
        self.outbound = original_outbound;
        self.outbound_journal = original_outbound_journal;
        self.effect_journal = original_effect_journal;
        self.outbound_sequence = original_sequence;
        self.next_message_id = original_message;
        self.next_effect_id = original_effect;
        execution?;

        let description = read_runtime_string(&candidate, "SAVEDATA_TEXT")?;
        let bytes = encode_scoped_save(
            &candidate.export_era_state(),
            candidate.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
            description,
            merge_structured_extensions(
                &self.save_extensions,
                candidate
                    .structured_extensions(StructuredScope::Ordinary)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?,
            self.traditional_save_format(),
        )
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        Ok((
            PendingCandidateCommit {
                state: candidate.into_candidate_state(),
                presentation: candidate_presentation,
                project_snapshot: candidate_project,
                message_skip: candidate_flags.0,
                skip_print: candidate_flags.1,
                user_defined_skip: candidate_flags.2,
                saved_skip: candidate_flags.3,
                effects,
            },
            bytes,
        ))
    }

    fn finish_candidate_save_failure(
        &mut self,
        continuation: CandidateSaveContinuation,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.pending_candidate_commit = None;
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.candidate_save_failed".into(),
                severity: DiagnosticSeverity::Warning,
                message: message.into(),
                source: None,
            }),
            None,
        )?;
        match continuation {
            CandidateSaveContinuation::Autosave => self.finish_builtin_autosave(false),
            CandidateSaveContinuation::SystemMenu { .. } => {
                self.system_menu = SystemMenuState::SaveSlots;
                self.render_slot_menu(true)
            }
        }
    }

    fn commit_candidate_save(
        &mut self,
        continuation: CandidateSaveContinuation,
    ) -> Result<(), RuntimeError> {
        let candidate = self.pending_candidate_commit.take().ok_or_else(|| {
            RuntimeError::Internal("candidate storage completion has no prepared state".into())
        })?;
        self.vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("candidate commit has no VM".into()))?
            .commit_candidate_state(candidate.state)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.presentation = candidate.presentation;
        self.project_snapshot = candidate.project_snapshot;
        self.message_skip = candidate.message_skip;
        self.skip_print = candidate.skip_print;
        self.user_defined_skip = candidate.user_defined_skip;
        self.saved_skip = candidate.saved_skip;
        self.emit_presentation()?;
        for effect in candidate.effects {
            self.emit_effect(effect)?;
        }
        match continuation {
            CandidateSaveContinuation::Autosave => self.finish_builtin_autosave(true),
            CandidateSaveContinuation::SystemMenu { request } => {
                self.system_menu_host_request = None;
                self.system_menu = SystemMenuState::Title;
                self.load_slot_paths.clear();
                self.occupied_slot_paths.clear();
                self.resume_storage_host(request, Vec::new())
            }
        }
    }

    fn issue_storage(
        &mut self,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        let request_id = self.allocate_request()?;
        self.operations.insert_storage(request_id, pending);
        self.set_phase(RuntimePhase::WaitingExternal)?;
        self.emit(
            RuntimeMessage::StorageRequest(StorageRequest {
                request_id,
                namespace,
                relative_path,
                operation,
                idempotency_key: format!(
                    "{}-{}-{}",
                    self.options.session_id.low, self.epoch.0, request_id
                ),
                deadline_ns: None,
            }),
            None,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn complete_storage(
        &mut self,
        message_id: u64,
        response: StorageResponse,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.operations.take_storage(response.request_id) else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "storage response has no pending request",
            );
        };
        match (pending, response.result) {
            (
                PendingStorage::CandidateSaveStat { slot, continuation },
                StorageResult::Metadata(metadata),
            ) => {
                let Some(revision) = metadata.revision else {
                    return self.finish_candidate_save_failure(
                        continuation,
                        "frontend stat omitted the revision required for an overwrite",
                    );
                };
                self.issue_candidate_clock(
                    slot,
                    StoragePrecondition::Revision(revision),
                    continuation,
                )
            }
            (
                PendingStorage::CandidateSaveStat { slot, continuation },
                StorageResult::Error { error },
            ) if error.kind == FrontendIoErrorKind::NotFound => {
                self.issue_candidate_clock(slot, StoragePrecondition::Missing, continuation)
            }
            (
                PendingStorage::CandidateSaveStat { continuation, .. },
                StorageResult::Error { error },
            ) => self.finish_candidate_save_failure(
                continuation,
                &format!("candidate stat failed: {error:?}"),
            ),
            (
                PendingStorage::CandidateSaveWrite { continuation },
                StorageResult::Written { .. },
            ) => self.commit_candidate_save(continuation),
            (
                PendingStorage::CandidateSaveWrite { continuation },
                StorageResult::Error { error },
            ) => self.finish_candidate_save_failure(
                continuation,
                &format!("candidate write failed: {error:?}"),
            ),
            (PendingStorage::HostFunctionWrite { request }, StorageResult::Written { .. })
            | (PendingStorage::HostStat { request }, StorageResult::Metadata(_)) => {
                self.resume_storage_host_value(request, VmValue::Integer(1), Vec::new())
            }
            (PendingStorage::HostFunctionWrite { request }, StorageResult::Error { .. })
            | (PendingStorage::HostStat { request }, StorageResult::Error { .. }) => {
                self.resume_storage_host_value(request, VmValue::Integer(0), Vec::new())
            }
            (PendingStorage::HostReadText { request }, StorageResult::Read { data, .. }) => {
                let text = std::str::from_utf8(data.as_slice())
                    .map(|value| value.trim_start_matches('\u{feff}').replace('\r', ""))
                    .unwrap_or_default();
                self.resume_storage_host_value(request, VmValue::String(text), Vec::new())
            }
            (PendingStorage::HostReadText { request }, StorageResult::Error { .. }) => {
                self.resume_storage_host_value(request, VmValue::String(String::new()), Vec::new())
            }
            (
                PendingStorage::HostListFiles {
                    request,
                    target,
                    strip_character_dat,
                },
                StorageResult::Listed { mut entries },
            ) => {
                entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
                let values = entries
                    .iter()
                    .map(|entry| {
                        if strip_character_dat {
                            entry
                                .relative_path
                                .strip_prefix("chara_")
                                .and_then(|value| value.strip_suffix(".dat"))
                                .unwrap_or(&entry.relative_path)
                                .to_owned()
                        } else {
                            entry.relative_path.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                let writes = self.file_list_writes(target, &values)?;
                self.resume_storage_host_value(
                    request,
                    VmValue::Integer(i64::try_from(entries.len()).unwrap_or(i64::MAX)),
                    writes,
                )
            }
            (PendingStorage::HostListFiles { request, .. }, StorageResult::Error { .. }) => {
                self.resume_storage_host_value(request, VmValue::Integer(-1), Vec::new())
            }
            (PendingStorage::HostWrite { request }, StorageResult::Written { .. })
            | (PendingStorage::HostDelete { request }, StorageResult::Deleted) => {
                self.resume_storage_host(request, Vec::new())
            }
            (PendingStorage::HostDelete { request }, StorageResult::Error { error })
                if error.kind == FrontendIoErrorKind::NotFound =>
            {
                // DELDATA is explicitly idempotent in the reference runtime.
                self.resume_storage_host(request, Vec::new())
            }
            (PendingStorage::HostLoadGlobal { request }, StorageResult::Error { error })
                if error.kind == FrontendIoErrorKind::NotFound =>
            {
                let writes = self.result_write(0)?;
                self.resume_storage_host(request, writes)
            }
            (PendingStorage::HostLoadGlobal { request }, StorageResult::Read { data, .. }) => {
                self.complete_global_load(request, data.as_slice())
            }
            (PendingStorage::HostLoadCharacters { request }, StorageResult::Read { data, .. }) => {
                self.complete_character_load(request, data.as_slice())
            }
            (PendingStorage::HostLoadCharacters { request }, StorageResult::Error { .. }) => {
                let writes = self.result_write(0)?;
                self.resume_storage_host(request, writes)
            }
            (PendingStorage::HostCheck { request, .. }, StorageResult::Error { error }) => {
                let status = if error.kind == FrontendIoErrorKind::NotFound {
                    1
                } else {
                    4
                };
                let writes = self.check_data_writes(status, &error.message)?;
                self.resume_storage_host(request, writes)
            }
            (PendingStorage::HostCheck { request, kind }, StorageResult::Read { data, .. }) => {
                let vm = self.vm.as_ref().ok_or_else(|| {
                    RuntimeError::Internal("save check completion has no VM".into())
                })?;
                let (status, description) =
                    match decode_scoped_save(data.as_slice(), vm.vm().artifact(), kind) {
                        Ok(decoded) => {
                            let game_base = &vm.vm().artifact().project_data.static_data.game_base;
                            if decoded.state.unique_code != game_base.unique_code {
                                (2, String::new())
                            } else if !vm
                                .vm()
                                .artifact()
                                .project_data
                                .save_load_context()
                                .compatibility
                                .accepts(decoded.state.unique_code, decoded.state.version)
                            {
                                (3, String::new())
                            } else {
                                (0, decoded.description)
                            }
                        }
                        Err(error) => (4, error.to_string()),
                    };
                let writes = self.check_data_writes(status, &description)?;
                self.resume_storage_host(request, writes)
            }
            (PendingStorage::HostLoadOrdinary { slot }, StorageResult::Read { data, .. }) => {
                self.complete_ordinary_load(slot, data.as_slice())
            }
            (PendingStorage::ListLoadSlots, StorageResult::Listed { entries }) => {
                self.open_slot_menu(message_id, entries, false)
            }
            (PendingStorage::ListSaveSlots, StorageResult::Listed { entries }) => {
                self.open_slot_menu(message_id, entries, true)
            }
            (
                PendingStorage::ScanMenuSlot {
                    save,
                    path,
                    remaining,
                },
                StorageResult::Read { data, .. },
            ) => {
                let vm = self
                    .vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("save menu scan has no VM".into()))?;
                let status = match decode_scoped_save(
                    data.as_slice(),
                    vm.vm().artifact(),
                    era_runtime_save::SaveFileKind::Normal,
                ) {
                    Ok(decoded) => {
                        let game = &vm.vm().artifact().project_data.static_data.game_base;
                        if decoded.state.unique_code != game.unique_code {
                            Err("different game".to_owned())
                        } else if !vm
                            .vm()
                            .artifact()
                            .project_data
                            .save_load_context()
                            .compatibility
                            .accepts(decoded.state.unique_code, decoded.state.version)
                        {
                            Err("different version".to_owned())
                        } else {
                            Ok(decoded.description)
                        }
                    }
                    Err(error) => Err(format!("corrupt: {error}")),
                };
                match status {
                    Ok(label) => {
                        self.slot_labels.insert(path, label);
                    }
                    Err(label) => {
                        self.invalid_slot_paths.insert(path.clone());
                        self.slot_labels.insert(path, label);
                    }
                }
                self.scan_next_menu_slot(save, remaining)
            }
            (
                PendingStorage::ScanMenuSlot {
                    save,
                    path,
                    remaining,
                },
                StorageResult::Error { error },
            ) => {
                if error.kind == FrontendIoErrorKind::NotFound {
                    self.occupied_slot_paths.remove(&path);
                    self.slot_revisions.remove(&path);
                } else {
                    self.invalid_slot_paths.insert(path.clone());
                    self.slot_labels
                        .insert(path, format!("I/O error: {}", error.message));
                }
                self.scan_next_menu_slot(save, remaining)
            }
            (PendingStorage::DeleteMenuSlot { save, path }, StorageResult::Deleted) => {
                self.occupied_slot_paths.remove(&path);
                self.slot_revisions.remove(&path);
                self.system_menu = if save {
                    SystemMenuState::SaveSlots
                } else {
                    SystemMenuState::LoadSlots
                };
                self.render_slot_menu(save)
            }
            (PendingStorage::DeleteMenuSlot { save, path }, StorageResult::Error { error })
                if error.kind == FrontendIoErrorKind::NotFound =>
            {
                self.occupied_slot_paths.remove(&path);
                self.slot_revisions.remove(&path);
                self.render_slot_menu(save)
            }
            (PendingStorage::DeleteMenuSlot { save, .. }, StorageResult::Error { error }) => {
                self.presentation.append_system_text(
                    format!("delete failed: {error:?}"),
                    SystemTextKey::InvalidValue,
                    Vec::new(),
                    true,
                );
                self.render_slot_menu(save)
            }
            (PendingStorage::ReadLoadSlot { slot }, StorageResult::Read { data, .. }) => {
                let vm = self.vm.as_ref().ok_or_else(|| {
                    RuntimeError::Internal("system load completion has no VM".into())
                })?;
                let valid = decode_scoped_save(
                    data.as_slice(),
                    vm.vm().artifact(),
                    era_runtime_save::SaveFileKind::Normal,
                )
                .ok()
                .is_some_and(|decoded| {
                    let game = &vm.vm().artifact().project_data.static_data.game_base;
                    decoded.state.unique_code == game.unique_code
                        && vm
                            .vm()
                            .artifact()
                            .project_data
                            .save_load_context()
                            .compatibility
                            .accepts(decoded.state.unique_code, decoded.state.version)
                });
                if !valid {
                    self.presentation.append_system_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        SystemTextKey::InvalidValue,
                        Vec::new(),
                        true,
                    );
                    return self.render_slot_menu(false);
                }
                self.system_menu_host_request = None;
                self.complete_ordinary_load(slot, data.as_slice())
            }
            (pending, StorageResult::Error { error }) => {
                if matches!(
                    pending,
                    PendingStorage::HostWrite { .. }
                        | PendingStorage::HostDelete { .. }
                        | PendingStorage::HostLoadOrdinary { .. }
                        | PendingStorage::HostLoadGlobal { .. }
                        | PendingStorage::HostLoadCharacters { .. }
                        | PendingStorage::HostCheck { .. }
                        | PendingStorage::HostFunctionWrite { .. }
                        | PendingStorage::HostReadText { .. }
                        | PendingStorage::HostStat { .. }
                        | PendingStorage::HostListFiles { .. }
                ) {
                    return self.fault(
                        FaultCode::ServiceFailure,
                        &format!("storage operation failed: {error:?}"),
                        None,
                    );
                }
                self.presentation.append_system_text(
                    format!(
                        "{}: {error:?}",
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue)
                    ),
                    SystemTextKey::InvalidValue,
                    Vec::new(),
                    true,
                );
                if matches!(
                    pending,
                    PendingStorage::ListLoadSlots
                        | PendingStorage::ListSaveSlots
                        | PendingStorage::ReadLoadSlot { .. }
                ) && self.system_menu_host_request.is_some()
                {
                    self.resume_system_menu_host()
                } else {
                    self.open_title_menu()
                }
            }
            _ => self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "storage response kind differs from its request",
            ),
        }
    }

    fn resume_storage_host(
        &mut self,
        request: erabasic_vm::HostRequestId,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    fn open_slot_menu(
        &mut self,
        message_id: u64,
        mut entries: Vec<StorageEntry>,
        save: bool,
    ) -> Result<(), RuntimeError> {
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if entries.iter().any(|entry| {
            era_runtime_protocol::validate_relative_path(&entry.relative_path).is_err()
        }) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "storage list contains an invalid relative path",
            );
        }
        self.occupied_slot_paths = entries
            .iter()
            .map(|entry| entry.relative_path.clone())
            .collect();
        self.slot_revisions = entries
            .into_iter()
            .filter_map(|entry| {
                entry
                    .revision
                    .map(|revision| (entry.relative_path, revision))
            })
            .collect();
        self.slot_labels.clear();
        self.invalid_slot_paths.clear();
        self.system_menu = if save {
            SystemMenuState::SaveSlots
        } else {
            SystemMenuState::LoadSlots
        };
        let mut remaining = self.occupied_slot_paths.iter().cloned().collect::<Vec<_>>();
        remaining.reverse();
        self.scan_next_menu_slot(save, remaining)
    }

    fn scan_next_menu_slot(
        &mut self,
        save: bool,
        mut remaining: Vec<String>,
    ) -> Result<(), RuntimeError> {
        let Some(path) = remaining.pop() else {
            return self.render_slot_menu(save);
        };
        self.issue_storage(
            PendingStorage::ScanMenuSlot {
                save,
                path: path.clone(),
                remaining,
            },
            StorageNamespace::Save,
            StorageOperation::Read,
            path,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn render_slot_menu(&mut self, save: bool) -> Result<(), RuntimeError> {
        let slot_count = self
            .project_snapshot
            .as_ref()
            .map_or(20, |snapshot| snapshot.save_slot_count)
            .max(20);
        let page_count = slot_count.div_ceil(20);
        self.system_menu_page = self.system_menu_page.min(page_count.saturating_sub(1));
        let start = self.system_menu_page.saturating_mul(20);
        let end = start.saturating_add(20).min(slot_count);
        self.load_slot_paths = (start..end).map(save_slot_path).collect();
        if !save && self.system_menu_page + 1 == page_count {
            self.load_slot_paths.push(save_slot_path(99));
        }
        let question = if save {
            SystemTextKey::SaveQuestion
        } else {
            SystemTextKey::LoadQuestion
        };
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, question),
            question,
            Vec::new(),
            false,
        );
        let mut choices = BTreeMap::new();
        for index in 0..self.load_slot_paths.len() {
            let path = self.load_slot_paths[index].clone();
            let occupied = self.occupied_slot_paths.contains(&path);
            let token = self.allocate_interaction();
            let label = if occupied {
                format!(
                    "{path}: {}",
                    self.slot_labels
                        .get(&path)
                        .map_or("(unreadable)", String::as_str)
                )
            } else {
                format!("{path}: ----")
            };
            self.presentation.append_system_button(
                label,
                SystemTextKey::SaveSlot,
                vec![SystemTextArgument::String(path)],
                token,
            );
            choices.insert(
                token,
                VmValue::Integer(i64::try_from(index).unwrap_or(i64::MAX).saturating_add(2)),
            );
            if occupied
                && self.storage_capabilities.delete
                && self.storage_capabilities.revisions
                && self
                    .slot_revisions
                    .contains_key(&self.load_slot_paths[index])
            {
                let delete = self.allocate_interaction();
                self.presentation.append_system_button(
                    format!("Delete {}", self.load_slot_paths[index]),
                    SystemTextKey::SaveSlot,
                    vec![SystemTextArgument::String(
                        self.load_slot_paths[index].clone(),
                    )],
                    delete,
                );
                choices.insert(
                    delete,
                    VmValue::Integer(-1_000 - i64::try_from(index).unwrap_or(i64::MAX)),
                );
            }
        }
        let back = self.allocate_interaction();
        self.presentation.append_system_button(
            localized_system_text(&self.selected_locale, SystemTextKey::Back),
            SystemTextKey::Back,
            Vec::new(),
            back,
        );
        choices.insert(back, VmValue::Integer(-1));
        if self.system_menu_page > 0 {
            let previous = self.allocate_interaction();
            self.presentation.append_system_button(
                "<".into(),
                SystemTextKey::Back,
                vec![SystemTextArgument::Integer(-1)],
                previous,
            );
            choices.insert(previous, VmValue::Integer(-2));
        }
        if self.system_menu_page + 1 < page_count {
            let next = self.allocate_interaction();
            self.presentation.append_system_button(
                ">".into(),
                SystemTextKey::Back,
                vec![SystemTextArgument::Integer(1)],
                next,
            );
            choices.insert(next, VmValue::Integer(-3));
        }
        let submission = self.allocate_interaction();
        let wait = self.system_wait(submission);
        self.open_wait(
            PendingInput {
                host_request: self.system_menu_host_request,
                wait,
                result_name: None,
                choices,
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    fn resume_system_menu_host(&mut self) -> Result<(), RuntimeError> {
        let Some(request) = self.system_menu_host_request.take() else {
            return self.open_title_menu();
        };
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths.clear();
        self.occupied_slot_paths.clear();
        self.resume_storage_host(request, Vec::new())
    }

    fn finish_builtin_autosave(&mut self, success: bool) -> Result<(), RuntimeError> {
        if !success {
            self.presentation.append_system_text(
                localized_system_text(&self.selected_locale, SystemTextKey::AutoSaveFailed),
                SystemTextKey::AutoSaveFailed,
                Vec::new(),
                false,
            );
        }
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("autosave completion has no VM".into()))?;
        self.controller.step = SystemStep::ShopShow;
        self.dispatch_system_function(&mut vm, "SHOW_SHOP", true)?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        self.renew_debug_grant()
    }

    fn resume_storage_host_value(
        &mut self,
        request: erabasic_vm::HostRequestId,
        value: VmValue,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: Some(value),
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    fn file_list_writes(
        &self,
        target: Option<PlaceDescriptor>,
        values: &[String],
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("file list completion has no VM".into()))?;
        let base = target.or_else(|| global_place_at(vm, "RESULTS", 0));
        let Some(base) = base else {
            return Ok(Vec::new());
        };
        let maximum = vm
            .vm()
            .artifact()
            .globals
            .iter()
            .find(|definition| definition.key == base.variable)
            .and_then(|definition| definition.dimensions.first())
            .and_then(|value| usize::try_from(*value).ok())
            .unwrap_or(0);
        Ok(values
            .iter()
            .take(maximum)
            .enumerate()
            .map(|(index, value)| {
                let mut target = base.clone();
                if let Some(last) = target.indices.last_mut() {
                    *last = u64::try_from(index).unwrap_or(u64::MAX);
                } else {
                    target
                        .indices
                        .push(u64::try_from(index).unwrap_or(u64::MAX));
                }
                HostWrite {
                    target,
                    value: VmValue::String(value.clone()),
                }
            })
            .collect())
    }

    fn result_write(&self, value: i64) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        Ok(global_place(vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(value),
                }]
            })
            .unwrap_or_default())
    }

    fn check_data_writes(
        &self,
        status: i64,
        description: &str,
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("save check completion has no VM".into()))?;
        let mut writes = Vec::new();
        if let Some(target) = global_place(vm, "RESULT") {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(status),
            });
        }
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(description.to_owned()),
            });
        }
        Ok(writes)
    }

    fn complete_global_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("global load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Global,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid global save: {error}")))?;
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::OverlayGlobal(Box::new(decoded.state)),
                StructuredScope::Global,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions =
            merge_opaque_extensions(&self.save_extensions, decoded.opaque_extensions);
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }

    fn complete_ordinary_load(&mut self, slot: u32, bytes: &[u8]) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid ordinary save: {error}")))?;
        let version = decoded.state.version;
        let description = decoded.description.clone();
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::RestoreOrdinary(Box::new(decoded.state)),
                StructuredScope::Ordinary,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let last_load = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::SetLastLoad {
                version,
                slot: i64::from(slot),
                text: description,
            })
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(last_load)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions = decoded.opaque_extensions;
        self.advance_epoch();
        self.controller.clear();
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        self.controller.prepare_load_sequence(vm.vm().artifact());
        if self.controller.is_complete() {
            self.continue_system_flow(&mut vm)?;
        } else {
            self.spawn_next_event(&mut vm)?;
        }
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }

    fn complete_character_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("character load has no VM".into()))?;
        let Ok(decoded) = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Character,
        ) else {
            let writes = global_place(&vm, "RESULT")
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: VmValue::Integer(0),
                    }]
                })
                .unwrap_or_default();
            commit_completion(
                &mut vm,
                request,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            )?;
            self.vm = Some(vm);
            return self.set_phase(RuntimePhase::Running);
        };
        let prepared = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::AppendCharacters(Box::new(
                decoded.state,
            )))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }

    #[allow(clippy::too_many_lines)]
    fn complete_service(
        &mut self,
        message_id: u64,
        response: ServiceResponse,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.operations.take_service(response.request_id) else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "service response has no pending request",
            );
        };
        if let PendingService::ProjectImageMetadata { relative_path } = pending {
            let result = match response.result {
                ServiceResult::Ready { payload } => {
                    let metadata: ImageMetadataResponse = decode_canonical(payload.as_slice())?;
                    let pending = self.pending_project_load.as_mut().ok_or_else(|| {
                        RuntimeError::Internal(
                            "image metadata completion has no pending project".into(),
                        )
                    })?;
                    let snapshot = match pending.reload.as_mut() {
                        Some(reload) => reload.build.snapshot.as_mut(),
                        None => self.project_snapshot.as_mut(),
                    }
                    .ok_or_else(|| {
                        RuntimeError::Internal(
                            "image metadata completion has no resource graph".into(),
                        )
                    })?;
                    snapshot
                        .resource_graph
                        .apply_metadata(&relative_path, metadata)
                }
                ServiceResult::Error { error } => Err(format!("{}: {}", error.code, error.message)),
            };
            let pending = self.pending_project_load.as_mut().ok_or_else(|| {
                RuntimeError::Internal("image metadata completion has no load report".into())
            })?;
            pending
                .remaining_metadata
                .remove(&relative_path.to_ascii_lowercase());
            if let Err(message) = result {
                pending.report.success = false;
                pending.report.diagnostics.push(ProtocolDiagnostic {
                    code: "runtime.invalid_image_metadata".into(),
                    severity: DiagnosticSeverity::Error,
                    message,
                    source: Some(era_runtime_protocol::SourceLocation {
                        relative_path,
                        byte_start: 0,
                        byte_end: 0,
                        line: None,
                        byte_column: None,
                    }),
                });
            }
            if pending.remaining_metadata.is_empty() {
                let mut pending = self.pending_project_load.take().expect("checked above");
                if let Some(mut reload) = pending.reload.take() {
                    reload.build.report = pending.report;
                    if reload.build.report.success {
                        return self.commit_project_reload(
                            pending.message_id,
                            reload.build,
                            reload.previous_phase,
                        );
                    }
                    self.emit(
                        RuntimeMessage::ProjectLoadReport(reload.build.report),
                        Some(pending.message_id),
                    )?;
                    return self.set_phase(reload.previous_phase);
                }
                return self.finish_project_load(pending.message_id, pending.report);
            }
            return Ok(());
        }
        if let PendingService::PlatformEffect { operation } = &pending {
            let failure = match response.result {
                ServiceResult::Ready { payload } if operation == OPEN_URL_OPERATION => {
                    let response: OpenUrlResponse = decode_canonical(payload.as_slice())?;
                    (!response.opened).then_some("frontend declined the URL request".to_owned())
                }
                ServiceResult::Ready { .. } => None,
                ServiceResult::Error { error } => {
                    Some(format!("{}: {}", error.code, error.message))
                }
            };
            if let Some(message) = failure {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.platform_effect_failed".into(),
                        severity: DiagnosticSeverity::Warning,
                        message,
                        source: None,
                    }),
                    Some(message_id),
                )?;
            }
            return Ok(());
        }
        if let PendingService::CandidateSaveClock {
            slot,
            precondition,
            continuation,
        } = pending
        {
            let payload = match response.result {
                ServiceResult::Ready { payload } => payload,
                ServiceResult::Error { error } => {
                    return self.finish_candidate_save_failure(
                        continuation,
                        &format!("candidate clock failed: {}: {}", error.code, error.message),
                    );
                }
            };
            let time: LocalDateTimeResponse = decode_canonical(payload.as_slice())?;
            let (candidate, bytes) = match self.prepare_candidate_save(time) {
                Ok(value) => value,
                Err(error) => {
                    return self.finish_candidate_save_failure(
                        continuation,
                        &format!("candidate SAVEINFO failed: {error}"),
                    );
                }
            };
            self.pending_candidate_commit = Some(candidate);
            return self.issue_storage(
                PendingStorage::CandidateSaveWrite { continuation },
                StorageNamespace::Save,
                StorageOperation::Write {
                    data: ProtocolBytes::new(bytes),
                    atomic_replace: true,
                    precondition,
                },
                save_slot_path(slot),
            );
        }
        if let PendingService::Host(ExternalCompletion::UpdateCheck { request, .. }) = &pending
            && let ServiceResult::Error { error } = &response.result
        {
            let result = if error.code.eq_ignore_ascii_case("network_unavailable") {
                5
            } else {
                3
            };
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("pending update check has no VM".into()))?;
            commit_host_result_write(vm, *request, result)?;
            return self.set_phase(RuntimePhase::Running);
        }
        let payload = match response.result {
            ServiceResult::Ready { payload } => payload,
            ServiceResult::Error { error } => {
                return self.fault(
                    FaultCode::ServiceFailure,
                    &format!("{}: {}", error.code, error.message),
                    None,
                );
            }
        };
        match pending {
            PendingService::StartEntropy => {
                let seed: RandomSeedResponse = decode_canonical(payload.as_slice())?;
                self.start_new_game(seed.seed)
            }
            PendingService::Host(completion) => {
                let mut writes = Vec::new();
                let value = match completion {
                    ExternalCompletion::GetKey {
                        key_code,
                        triggered,
                        ..
                    } => {
                        let state: GetKeyStateResponse = decode_canonical(payload.as_slice())?;
                        let index = usize::from(key_code);
                        let previous = self.key_toggle_state[index];
                        let current = u8::from(state.toggle_state) + 1;
                        self.key_toggle_state[index] = current;
                        Some(VmValue::Integer(i64::from(
                            state.frontend_active
                                && state.pressed
                                && (!triggered || previous != current),
                        )))
                    }
                    ExternalCompletion::LocalDateTime {
                        operation, result, ..
                    } => {
                        let time: LocalDateTimeResponse = decode_canonical(payload.as_slice())?;
                        if result.is_none() {
                            let vm = self.vm.as_ref().ok_or_else(|| {
                                RuntimeError::Internal("pending clock service has no VM".into())
                            })?;
                            if let Some(target) = global_place(vm, "RESULT") {
                                writes.push(HostWrite {
                                    target,
                                    value: VmValue::Integer(calendar_number(time)),
                                });
                            }
                            if let Some(target) = global_place(vm, "RESULTS") {
                                writes.push(HostWrite {
                                    target,
                                    value: VmValue::String(calendar_string(time)),
                                });
                            }
                            None
                        } else {
                            Some(match operation {
                                ClockOperation::Time => VmValue::Integer(calendar_number(time)),
                                ClockOperation::Times => VmValue::String(calendar_string(time)),
                                ClockOperation::Millisecond => {
                                    VmValue::Integer(milliseconds_since_year_one(time))
                                }
                                ClockOperation::Second => {
                                    VmValue::Integer(milliseconds_since_year_one(time) / 1_000)
                                }
                            })
                        }
                    }
                    ExternalCompletion::SpritePixel { .. } => {
                        let pixel: ImagePixelResponse = decode_canonical(payload.as_slice())?;
                        Some(VmValue::Integer(i64::from(pixel.argb)))
                    }
                    ExternalCompletion::UpdateCheck { request } => {
                        let update: UpdateCheckResponse = decode_canonical(payload.as_slice())?;
                        if update.remote_version.is_empty() || update.download_url.is_empty() {
                            let vm = self.vm.as_mut().ok_or_else(|| {
                                RuntimeError::Internal("pending update check has no VM".into())
                            })?;
                            commit_host_result_write(vm, request, 3)?;
                            return self.set_phase(RuntimePhase::Running);
                        }
                        let current_version = self
                            .vm
                            .as_ref()
                            .map(|vm| {
                                &vm.vm()
                                    .artifact()
                                    .project_data
                                    .static_data
                                    .game_base
                                    .version_name
                            })
                            .cloned()
                            .unwrap_or_default();
                        if update.remote_version == current_version {
                            let vm = self.vm.as_mut().ok_or_else(|| {
                                RuntimeError::Internal("pending update check has no VM".into())
                            })?;
                            commit_host_result_write(vm, request, 0)?;
                            return self.set_phase(RuntimePhase::Running);
                        }
                        return self.open_update_prompt(
                            request,
                            &update.remote_version,
                            update.download_url,
                        );
                    }
                };
                let host_request = match completion {
                    ExternalCompletion::GetKey { request: id, .. }
                    | ExternalCompletion::LocalDateTime { request: id, .. }
                    | ExternalCompletion::SpritePixel { request: id }
                    | ExternalCompletion::UpdateCheck { request: id, .. } => id,
                };
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("pending service has no VM".into()))?;
                commit_completion(
                    vm,
                    host_request,
                    VmHostCompletion::Ready(HostReady { value, writes }),
                )?;
                self.set_phase(RuntimePhase::Running)
            }
            PendingService::ProjectImageMetadata { .. }
            | PendingService::PlatformEffect { .. }
            | PendingService::CandidateSaveClock { .. } => {
                unreachable!("handled above")
            }
        }
    }

    fn open_update_prompt(
        &mut self,
        request: erabasic_vm::HostRequestId,
        remote_version: &str,
        download_url: String,
    ) -> Result<(), RuntimeError> {
        self.presentation.append_text(
            format!("New version {remote_version} is available: {download_url}"),
            false,
        );
        let no = self.allocate_interaction();
        let yes = self.allocate_interaction();
        self.presentation.append_button("No".into(), no, None);
        self.presentation.append_button("Yes".into(), yes, None);
        let submission = self.allocate_interaction();
        let pending = PendingInput {
            host_request: Some(request),
            wait: InputWait {
                wait_id: self.allocate_wait(),
                kind: WaitKind::IntegerButton,
                stability: WaitStability::Transient,
                one_input: false,
                stop_message_skip: false,
                system_input: false,
                mouse_input: false,
                default_value: None,
                deadline_ns: None,
                display_time: false,
                timeout_message: None,
                submission_token: submission,
                countdown_remaining_ms: None,
            },
            result_name: Some("RESULT".into()),
            choices: BTreeMap::from([(no, VmValue::Integer(1)), (yes, VmValue::Integer(2))]),
            timeout_duration_ns: None,
            post_input: Some(PostInputAction::OpenUrl {
                url: download_url,
                trigger_value: 2,
            }),
        };
        self.open_wait(pending, false)
    }

    fn complete_input(
        &mut self,
        message_id: u64,
        input: FrontendInput,
    ) -> Result<(), RuntimeError> {
        let Some(wait_id) = self
            .operations
            .active_input()
            .map(|pending| pending.wait.wait_id)
        else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "no input is pending",
            );
        };
        if wait_id != input.wait_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "input wait identity is stale",
            );
        }
        let observed_time = self.observe_frontend_time(input.monotonic_time_ns);
        let pending = self.operations.active_input().expect("checked above");
        if pending
            .wait
            .deadline_ns
            .is_some_and(|deadline| observed_time > deadline)
        {
            return self.advance_time(
                message_id,
                AdvanceTime {
                    monotonic_time_ns: input.monotonic_time_ns,
                },
            );
        }
        self.message_skip = input.message_skip;
        let Some(submission) = input_value(pending, input.token, input.intent) else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "input value does not match the active wait",
            );
        };
        self.finish_input(submission, false)
    }

    fn advance_time(&mut self, _message_id: u64, time: AdvanceTime) -> Result<(), RuntimeError> {
        self.observe_frontend_time(time.monotonic_time_ns);
        let ready_delays = self.operations.take_ready_delays(self.logical_time_ns);
        for request in ready_delays {
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("pending AWAIT has no VM".into()))?;
            commit_completion(vm, request, VmHostCompletion::Ready(HostReady::empty()))?;
            self.set_phase(RuntimePhase::Running)?;
        }
        let timed_out = self
            .operations
            .active_input()
            .and_then(|pending| pending.wait.deadline_ns)
            .is_some_and(|deadline| self.logical_time_ns >= deadline);
        if !timed_out
            && let Some(pending) = self.operations.active_input_mut()
            && pending.wait.display_time
            && let Some(deadline) = pending.wait.deadline_ns
        {
            let remaining = deadline
                .saturating_sub(self.logical_time_ns)
                .saturating_add(999_999)
                / 1_000_000;
            pending.wait.countdown_remaining_ms = Some(remaining);
            let wait = pending.wait.clone();
            self.presentation.set_wait(Some(wait.clone()));
            self.emit(RuntimeMessage::WaitChanged(WaitChange::Updated(wait)), None)?;
            self.emit_presentation()?;
        }
        if timed_out {
            let pending = self.operations.active_input().expect("checked above");
            if let Some(message) = &pending.wait.timeout_message {
                self.presentation.append_text(message.clone(), false);
            }
            let submission = if pending.wait.kind == WaitKind::PrimitiveMouseKey {
                InputSubmission::Primitive(PrimitiveResult {
                    fields: [4, 0, 0, 0, 0],
                    selection: None,
                })
            } else {
                InputSubmission::Value(
                    pending
                        .wait
                        .default_value
                        .as_ref()
                        .map_or(VmValue::Integer(0), protocol_to_vm),
                )
            };
            self.finish_input(submission, true)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn finish_input(
        &mut self,
        submission: InputSubmission,
        timed_out: bool,
    ) -> Result<(), RuntimeError> {
        let pending = self
            .operations
            .take_active_input()
            .ok_or_else(|| RuntimeError::Internal("input wait disappeared".into()))?;
        if pending.wait.system_input {
            let InputSubmission::Value(value) = submission else {
                return Err(RuntimeError::Internal(
                    "system input cannot accept primitive fields".into(),
                ));
            };
            return self.finish_system_input(pending, &value);
        }
        let request = pending
            .host_request
            .ok_or_else(|| RuntimeError::Internal("VM wait has no host request".into()))?;
        let post_url = match (&pending.post_input, &submission) {
            (
                Some(PostInputAction::OpenUrl { url, trigger_value }),
                InputSubmission::Value(VmValue::Integer(value)),
            ) if value == trigger_value => Some(url.clone()),
            _ => None,
        };
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("input wait has no VM".into()))?;
        let mut writes = Vec::new();
        match submission {
            InputSubmission::Value(value) => {
                let result_name = if pending.wait.kind == WaitKind::AnyValue {
                    Some(match &value {
                        VmValue::String(_) => "RESULTS",
                        _ => "RESULT",
                    })
                } else {
                    pending.result_name.as_deref()
                };
                if let Some(target) = result_name.and_then(|name| global_place(vm, name)) {
                    writes.push(HostWrite { target, value });
                }
            }
            InputSubmission::Primitive(primitive) => {
                for (index, value) in primitive.fields.into_iter().enumerate() {
                    if let Some(target) = global_place_at(vm, "RESULT", index) {
                        writes.push(HostWrite {
                            target,
                            value: VmValue::Integer(i64::from(value)),
                        });
                    }
                }
                let result_5 = match primitive.selection {
                    Some(VmValue::Integer(value)) => value,
                    Some(VmValue::String(value)) => {
                        if let Some(target) = global_place(vm, "RESULTS") {
                            writes.push(HostWrite {
                                target,
                                value: VmValue::String(value),
                            });
                        }
                        0
                    }
                    None => 0,
                    Some(VmValue::IntegerPlace(_) | VmValue::StringPlace(_)) => {
                        return Err(RuntimeError::Internal(
                            "an interaction token resolved to a VM place".into(),
                        ));
                    }
                };
                if let Some(target) = global_place_at(vm, "RESULT", 5) {
                    writes.push(HostWrite {
                        target,
                        value: VmValue::Integer(result_5),
                    });
                }
            }
        }
        // ISTIMEOUT is only changed by a timed input completion; untimed waits leave it sticky.
        if pending.wait.deadline_ns.is_some()
            && let Some(target) = global_place(vm, "ISTIMEOUT")
        {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(i64::from(timed_out)),
            });
        }
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        let pause_next_wait = !vm.has_runnable_fibers();
        self.close_wait(pending.wait.wait_id)?;
        if let Some(next) = self.operations.pop_queued_input() {
            self.activate_wait(next, pause_next_wait)?;
        } else {
            self.set_phase(RuntimePhase::Running)?;
        }
        if let Some(url) = post_url {
            self.issue_platform_effect(
                ServiceKind::OpenUrl,
                OPEN_URL_OPERATION,
                OPEN_URL_OPERATION_VERSION,
                &OpenUrlRequest { url },
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn finish_system_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        if self.controller.step != SystemStep::None && self.system_menu_host_request.is_none() {
            return self.finish_flow_input(pending, value);
        }
        match (self.system_menu, value) {
            (SystemMenuState::Title, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                self.controller.flow = Some(SystemFlow::First);
                if !self
                    .controller
                    .prepare_event(vm.vm().artifact(), "EVENTFIRST")
                {
                    return Err(RuntimeError::Internal("EVENTFIRST is not defined".into()));
                }
                let entry = self.controller.next().expect("prepared EVENTFIRST entry");
                let fiber = vm
                    .spawn_entry(entry, Vec::new())
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                self.controller.started(fiber);
                self.set_phase(RuntimePhase::Running)
            }
            (SystemMenuState::Title, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                if self
                    .controller
                    .prepare_function(vm.vm().artifact(), "TITLE_LOADGAME")
                {
                    self.controller.flow = Some(SystemFlow::Title);
                    self.controller.step = SystemStep::TitleLoadOverride;
                    let entry = self
                        .controller
                        .next()
                        .expect("prepared TITLE_LOADGAME entry");
                    let fiber = vm
                        .spawn_entry(entry, Vec::new())
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    self.controller.started(fiber);
                    return self.set_phase(RuntimePhase::Running);
                }
                self.issue_storage(
                    PendingStorage::ListLoadSlots,
                    StorageNamespace::Save,
                    StorageOperation::List {
                        pattern: Some("save*.sav".into()),
                        recursive: false,
                    },
                    String::new(),
                )
            }
            (
                SystemMenuState::LoadSlots | SystemMenuState::SaveSlots,
                VmValue::Integer(selection),
            ) if *selection <= -1_000 => {
                let index = usize::try_from(selection.saturating_neg().saturating_sub(1_000))
                    .unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown delete slot");
                };
                let Some(revision) = self.slot_revisions.get(&path).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidState,
                        "save slot revision is unavailable",
                    );
                };
                let save = self.system_menu == SystemMenuState::SaveSlots;
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::DeleteMenuSlot {
                        save,
                        path: path.clone(),
                    },
                    StorageNamespace::Save,
                    StorageOperation::Delete {
                        precondition: StoragePrecondition::Revision(revision),
                    },
                    path,
                )
            }
            (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots, VmValue::Integer(-1)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.resume_system_menu_host()
            }
            (
                menu @ (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots),
                VmValue::Integer(-2 | -3),
            ) => {
                self.close_wait(pending.wait.wait_id)?;
                if value == &VmValue::Integer(-2) {
                    self.system_menu_page = self.system_menu_page.saturating_sub(1);
                } else {
                    self.system_menu_page = self.system_menu_page.saturating_add(1);
                }
                self.render_slot_menu(menu == SystemMenuState::SaveSlots)
            }
            (SystemMenuState::LoadSlots, VmValue::Integer(selection)) if *selection >= 2 => {
                let index = usize::try_from(*selection - 2).unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                };
                if !self.occupied_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "save slot is empty");
                }
                if self.invalid_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "save slot is incompatible or corrupt",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                let slot = parse_save_slot(&path).ok_or_else(|| {
                    RuntimeError::Internal("system load menu generated an invalid slot path".into())
                })?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::SaveSlots, VmValue::Integer(selection)) if *selection >= 2 => {
                let index = usize::try_from(*selection - 2).unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                };
                let slot = parse_save_slot(&path).ok_or_else(|| {
                    RuntimeError::Internal("system save menu generated an invalid slot path".into())
                })?;
                self.close_wait(pending.wait.wait_id)?;
                if self.occupied_slot_paths.contains(&path) {
                    self.system_menu = SystemMenuState::ConfirmOverwrite { slot };
                    self.presentation.append_system_text(
                        localized_system_text(
                            &self.selected_locale,
                            SystemTextKey::OverwriteQuestion,
                        ),
                        SystemTextKey::OverwriteQuestion,
                        vec![SystemTextArgument::Integer(i64::from(slot))],
                        false,
                    );
                    let yes = self.allocate_interaction();
                    let no = self.allocate_interaction();
                    self.presentation.append_system_button(
                        "Yes".into(),
                        SystemTextKey::OverwriteQuestion,
                        vec![SystemTextArgument::Integer(0)],
                        yes,
                    );
                    self.presentation.append_system_button(
                        "No".into(),
                        SystemTextKey::OverwriteQuestion,
                        vec![SystemTextArgument::Integer(1)],
                        no,
                    );
                    let submission = self.allocate_interaction();
                    let wait = self.system_wait(submission);
                    return self.open_wait(
                        PendingInput {
                            host_request: self.system_menu_host_request,
                            wait,
                            result_name: None,
                            choices: BTreeMap::from([
                                (yes, VmValue::Integer(0)),
                                (no, VmValue::Integer(1)),
                            ]),
                            timeout_duration_ns: None,
                            post_input: None,
                        },
                        true,
                    );
                }
                self.begin_system_menu_candidate(slot)
            }
            (SystemMenuState::ConfirmOverwrite { slot }, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.begin_system_menu_candidate(slot)
            }
            (SystemMenuState::ConfirmOverwrite { .. }, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.system_menu = SystemMenuState::SaveSlots;
                self.render_slot_menu(true)
            }
            _ => {
                if self.presentation.last_line_is_temporary()
                    && self.presentation.last_line_is_empty()
                {
                    self.presentation.delete_last_lines(2);
                    self.presentation.append_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        true,
                    );
                } else {
                    self.presentation
                        .replace_last_temporary(localized_system_text(
                            &self.selected_locale,
                            SystemTextKey::InvalidValue,
                        ));
                }
                self.operations.restore_active_input(pending);
                self.emit_presentation()?;
                self.reject(
                    0,
                    CommandErrorCode::InvalidValue,
                    "unknown system menu item",
                )
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn finish_flow_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        let VmValue::Integer(selection) = value else {
            self.operations.restore_active_input(pending);
            return self.reject(
                0,
                CommandErrorCode::InvalidValue,
                "system input must be integer",
            );
        };
        let previous_choices = pending.choices.clone();
        self.close_wait(pending.wait.wait_id)?;
        self.set_phase(RuntimePhase::Running)?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("system flow input has no VM".into()))?;
        let result = match self.controller.step {
            SystemStep::TrainShowUser => {
                if let Some(command) = usize::try_from(*selection)
                    .ok()
                    .and_then(|index| self.controller.train_commands.get(index))
                    .copied()
                {
                    self.controller.selected_command = Some(command);
                    write_runtime_integer(&mut vm, "SELECTCOM", &[], None, command)?;
                    fill_runtime_variable(&mut vm, "NOWEX", VmValue::Integer(0), true)?;
                    self.controller.step = SystemStep::TrainEventCom;
                    if self.dispatch_system_event(&mut vm, "EVENTCOM")? {
                        Ok(())
                    } else {
                        self.continue_system_flow(&mut vm)
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.controller.step = SystemStep::TrainUserCom;
                    self.dispatch_system_function(&mut vm, "USERCOM", true)?;
                    Ok(())
                }
            }
            SystemStep::AblupShowSelect => {
                self.controller.step = SystemStep::AblupAction;
                if (0..100).contains(selection) {
                    if self.dispatch_system_function(
                        &mut vm,
                        &format!("ABLUP{selection}"),
                        false,
                    )? {
                        Ok(())
                    } else {
                        self.presentation
                            .replace_last_temporary(localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::InvalidValue,
                            ));
                        self.command_intents = previous_choices.clone();
                        self.open_system_command_wait()
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.dispatch_system_function(&mut vm, "USERABLUP", true)?;
                    Ok(())
                }
            }
            SystemStep::ShopShow => {
                let maximum = self
                    .project_snapshot
                    .as_ref()
                    .map_or(100, |snapshot| snapshot.maximum_shop_items);
                if *selection >= 0 && *selection < i64::from(maximum) {
                    let purchase = purchase_item(
                        &mut vm,
                        usize::try_from(*selection).unwrap_or(usize::MAX),
                        maximum,
                    )?;
                    match purchase {
                        PurchaseResult::Purchased => {
                            self.controller.step = SystemStep::ShopAction;
                            if !self.dispatch_system_event(&mut vm, "EVENTBUY")? {
                                self.continue_system_flow(&mut vm)?;
                            }
                            Ok(())
                        }
                        PurchaseResult::OutOfStock | PurchaseResult::NotEnoughMoney => {
                            let key = if purchase == PurchaseResult::NotEnoughMoney {
                                SystemTextKey::NotEnoughMoney
                            } else {
                                SystemTextKey::OutOfStock
                            };
                            self.presentation
                                .replace_last_temporary(localized_system_text(
                                    &self.selected_locale,
                                    key,
                                ));
                            self.command_intents = previous_choices.clone();
                            self.open_system_command_wait()
                        }
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.controller.step = SystemStep::ShopAction;
                    self.dispatch_system_function(&mut vm, "USERSHOP", true)?;
                    Ok(())
                }
            }
            _ => Err(RuntimeError::Internal(
                "system flow received input outside an input step".into(),
            )),
        };
        self.vm = Some(vm);
        result
    }

    fn spawn_next_event(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        if let Some(entry) = self.controller.next() {
            let fiber = vm
                .spawn_entry(entry, Vec::new())
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.started(fiber);
        }
        Ok(())
    }

    fn begin_flow(&mut self, vm: &mut RuntimeVm, flow: SystemFlow) -> Result<(), RuntimeError> {
        self.message_skip = false;
        if flow == SystemFlow::Train {
            reset_training_state(vm)?;
            self.controller.train_scan = 0;
            self.controller.train_commands.clear();
            self.controller.continuous_commands.clear();
            self.controller.continuous_train = false;
        }
        self.controller.step = match flow {
            SystemFlow::Train => SystemStep::TrainEvent,
            SystemFlow::Ablup => SystemStep::AblupShowJuel,
            SystemFlow::Shop => SystemStep::ShopEvent,
            _ => SystemStep::None,
        };
        let (entry, event, required) = match flow {
            SystemFlow::Title => ("SYSTEM_TITLE", false, false),
            SystemFlow::First => ("EVENTFIRST", true, true),
            SystemFlow::Train => ("EVENTTRAIN", true, false),
            SystemFlow::AfterTrain => ("EVENTEND", true, true),
            SystemFlow::Ablup => ("SHOW_JUEL", false, true),
            SystemFlow::TurnEnd => ("EVENTTURNEND", true, true),
            SystemFlow::Shop => ("EVENTSHOP", true, false),
            SystemFlow::Normal => {
                return self.fault(
                    FaultCode::VmFault,
                    "NORMAL is an internal system state and is not a BEGIN target",
                    None,
                );
            }
        };
        if event {
            if self.controller.prepare_event(vm.vm().artifact(), entry) {
                return self.spawn_next_event(vm);
            }
        } else if self.controller.prepare_function(vm.vm().artifact(), entry) {
            return self.spawn_next_event(vm);
        }
        if required {
            self.fault(
                FaultCode::VmFault,
                &format!("required system function {entry} is not defined"),
                None,
            )
        } else if self.controller.step != SystemStep::None {
            self.continue_system_flow(vm)
        } else {
            Ok(())
        }
    }

    fn dispatch_system_function(
        &mut self,
        vm: &mut RuntimeVm,
        name: &str,
        required: bool,
    ) -> Result<bool, RuntimeError> {
        if self.controller.prepare_function(vm.vm().artifact(), name) {
            self.spawn_next_event(vm)?;
            return Ok(true);
        }
        if required {
            self.fault(
                FaultCode::VmFault,
                &format!("required system function {name} is not defined"),
                None,
            )?;
        }
        Ok(false)
    }

    fn dispatch_system_event(
        &mut self,
        vm: &mut RuntimeVm,
        name: &str,
    ) -> Result<bool, RuntimeError> {
        if self.controller.prepare_event(vm.vm().artifact(), name) {
            self.spawn_next_event(vm)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn open_system_command_wait(&mut self) -> Result<(), RuntimeError> {
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        wait.kind = WaitKind::IntegerValue;
        let choices = std::mem::take(&mut self.command_intents);
        self.reusable_system_intents.clone_from(&choices);
        self.open_wait(
            PendingInput {
                host_request: None,
                wait,
                result_name: None,
                choices,
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn continue_system_flow(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        match self.controller.step {
            SystemStep::TrainEvent => {
                let next = read_runtime_integer(vm, "NEXTCOM", &[], None)?;
                if next >= 0 {
                    write_runtime_integer(vm, "SELECTCOM", &[], None, next)?;
                    write_runtime_integer(vm, "NEXTCOM", &[], None, 0)?;
                    self.controller.selected_command = Some(next);
                    fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
                    self.controller.step = SystemStep::TrainEventCom;
                    if !self.dispatch_system_event(vm, "EVENTCOM")? {
                        return self.continue_system_flow(vm);
                    }
                } else {
                    self.controller.step = SystemStep::TrainShowStatus;
                    self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                }
            }
            SystemStep::TrainShowStatus => {
                self.controller.step = SystemStep::TrainComAble;
                self.controller.train_scan = 0;
                self.controller.train_commands.clear();
                return self.prepare_next_comable(vm);
            }
            SystemStep::TrainComAble => {
                let command = self.controller.train_scan.saturating_sub(1);
                if read_runtime_integer(vm, "RESULT", &[], None)? != 0 {
                    self.controller
                        .train_commands
                        .push(i64::try_from(command).unwrap_or(i64::MAX));
                }
                return self.prepare_next_comable(vm);
            }
            SystemStep::TrainShowUser if self.controller.continuous_train => {
                reset_after_show_user(vm)?;
                if let Some(command) = self.controller.continuous_commands.pop_front() {
                    if self.controller.train_commands.contains(&command) {
                        self.controller.selected_command = Some(command);
                        write_runtime_integer(vm, "SELECTCOM", &[], None, command)?;
                        fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
                        self.controller.step = SystemStep::TrainEventCom;
                        if !self.dispatch_system_event(vm, "EVENTCOM")? {
                            return self.continue_system_flow(vm);
                        }
                    } else {
                        self.controller.step = SystemStep::TrainShowStatus;
                        self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                    }
                } else {
                    self.controller.continuous_train = false;
                    self.controller.step = SystemStep::TrainShowStatus;
                    if !self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
                        self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                    }
                }
            }
            SystemStep::TrainShowUser => {
                reset_after_show_user(vm)?;
                return self.open_system_command_wait();
            }
            SystemStep::AblupShowSelect | SystemStep::ShopShow => {
                return self.open_system_command_wait();
            }
            SystemStep::TrainUserCom | SystemStep::TrainEventComEnd => {
                self.controller.step = SystemStep::TrainShowStatus;
                self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
            }
            SystemStep::TrainEventCom => {
                let command = self.controller.selected_command.ok_or_else(|| {
                    RuntimeError::Internal("training command selection disappeared".into())
                })?;
                self.controller.step = SystemStep::TrainCommand;
                self.dispatch_system_function(vm, &format!("COM{command}"), true)?;
            }
            SystemStep::TrainCommand => {
                let result = read_runtime_integer(vm, "RESULT", &[], None)?;
                if result == 0 {
                    self.controller.step = SystemStep::TrainEventComEnd;
                    if !self.dispatch_system_event(vm, "EVENTCOMEND")? {
                        return self.continue_system_flow(vm);
                    }
                } else {
                    self.controller.step = SystemStep::TrainSourceCheck;
                    self.dispatch_system_function(vm, "SOURCE_CHECK", true)?;
                }
            }
            SystemStep::TrainSourceCheck => {
                fill_runtime_variable(vm, "SOURCE", VmValue::Integer(0), true)?;
                self.controller.step = SystemStep::TrainEventComEnd;
                if !self.dispatch_system_event(vm, "EVENTCOMEND")? {
                    return self.continue_system_flow(vm);
                }
            }
            SystemStep::AblupShowJuel => {
                self.controller.step = SystemStep::AblupShowSelect;
                self.dispatch_system_function(vm, "SHOW_ABLUP_SELECT", true)?;
            }
            SystemStep::AblupAction => {
                if self.presentation.last_line_is_temporary() {
                    self.command_intents
                        .clone_from(&self.reusable_system_intents);
                    return self.open_system_command_wait();
                }
                self.controller.step = SystemStep::AblupShowJuel;
                self.dispatch_system_function(vm, "SHOW_JUEL", true)?;
            }
            SystemStep::ShopEvent => {
                if self
                    .project_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.auto_save)
                {
                    self.controller.step = SystemStep::ShopAutosave;
                    if !self.dispatch_system_function(vm, "SYSTEM_AUTOSAVE", false)? {
                        return self.begin_candidate_save(
                            vm,
                            99,
                            CandidateSaveContinuation::Autosave,
                        );
                    }
                } else {
                    self.controller.step = SystemStep::ShopShow;
                    self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
                }
            }
            SystemStep::ShopAutosave | SystemStep::ShopAction | SystemStep::PostLoadShop => {
                self.controller.step = SystemStep::ShopShow;
                self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
            }
            SystemStep::TitleLoadOverride => {
                self.controller.step = SystemStep::None;
                return self.open_title_menu();
            }
            SystemStep::None => {}
        }
        Ok(())
    }

    fn prepare_next_comable(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        let names = vm
            .vm()
            .artifact()
            .project_data
            .static_data
            .name_tables
            .get(&erabasic_data::NameTableKind::Train)
            .map(|table| table.names.clone())
            .unwrap_or_default();
        let default_enabled = vm
            .vm()
            .artifact()
            .project_data
            .static_data
            .replace
            .com_able_default
            != 0;
        while self.controller.train_scan < names.len() {
            let command = self.controller.train_scan;
            self.controller.train_scan += 1;
            if names[command].is_none() {
                continue;
            }
            if self.dispatch_system_function(vm, &format!("COM_ABLE{command}"), false)? {
                return Ok(());
            }
            if default_enabled {
                self.controller
                    .train_commands
                    .push(i64::try_from(command).unwrap_or(i64::MAX));
            }
        }
        for (display, command) in self
            .controller
            .train_commands
            .clone()
            .into_iter()
            .enumerate()
        {
            let name = usize::try_from(command)
                .ok()
                .and_then(|index| names.get(index))
                .and_then(Option::as_deref)
                .unwrap_or("");
            let token = self.allocate_interaction();
            self.presentation
                .append_button(format!("{name}[{display:>3}]"), token, None);
            self.command_intents.insert(
                token,
                VmValue::Integer(i64::try_from(display).unwrap_or(i64::MAX)),
            );
        }
        self.controller.step = SystemStep::TrainShowUser;
        self.dispatch_system_function(vm, "SHOW_USERCOM", true)?;
        Ok(())
    }

    fn open_wait(
        &mut self,
        pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if self.operations.active_input().is_some() {
            self.operations.queue_input(pending);
            return Ok(());
        }
        self.activate_wait(pending, pause_runtime)
    }

    fn activate_wait(
        &mut self,
        mut pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if let Some(duration) = pending.timeout_duration_ns {
            pending.wait.deadline_ns = Some(self.logical_time_ns.saturating_add(duration));
            if pending.wait.display_time {
                pending.wait.countdown_remaining_ms = Some(duration / 1_000_000);
            }
        }
        self.presentation.set_wait(Some(pending.wait.clone()));
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Opened(pending.wait.clone())),
            None,
        )?;
        self.operations.activate_input(pending);
        self.emit_presentation()?;
        if pause_runtime {
            self.set_phase(RuntimePhase::WaitingInput)
        } else {
            Ok(())
        }
    }

    fn close_wait(&mut self, wait_id: u64) -> Result<(), RuntimeError> {
        self.presentation.set_wait(None);
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Closed(wait_id)),
            None,
        )?;
        self.emit_presentation()
    }

    #[allow(clippy::too_many_lines)]
    fn export_state(
        &mut self,
        message_id: u64,
        request: StateExportRequest,
    ) -> Result<(), RuntimeError> {
        let stable_wait = self.operations.active_input().is_some_and(|pending| {
            pending.wait.stability == WaitStability::StableInput
                && pending.wait.deadline_ns.is_none()
        });
        let mut reasons = Vec::new();
        if self.phase != RuntimePhase::WaitingInput || !stable_wait {
            reasons.push(SnapshotIneligibleReason::StableWaitRequired);
        }
        if self.operations.has_transient_external() || !self.effect_journal.is_empty() {
            reasons.push(SnapshotIneligibleReason::ExternalOperationPending);
        }
        if request.kind == StateExportKind::VmSnapshot && !self.operations.is_snapshot_stable() {
            reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
        }
        let result = if reasons.is_empty() {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("save export has no VM".into()))?;
            let bytes = match request.kind {
                StateExportKind::TraditionalSave => encode_era_save(
                    &vm.export_era_state(),
                    vm.vm().artifact(),
                    String::new(),
                    merge_structured_extensions(
                        &self.save_extensions,
                        vm.structured_extensions(StructuredScope::Ordinary)
                            .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                    )
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                    self.traditional_save_format(),
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                StateExportKind::VmSnapshot => {
                    let vm_snapshot = match vm.snapshot() {
                        Ok(snapshot) => snapshot.encode().map_err(|error| {
                            RuntimeError::Internal(format!("VM snapshot encode failed: {error}"))
                        })?,
                        Err(_) => {
                            return self.emit(
                                RuntimeMessage::StateExportReady(StateExportReady {
                                    kind: request.kind,
                                    result: StateExportResult::Ineligible {
                                        reasons: vec![
                                            SnapshotIneligibleReason::SnapshotStateUnavailable,
                                        ],
                                    },
                                }),
                                Some(message_id),
                            );
                        }
                    };
                    let project = self.project_snapshot.as_ref().ok_or_else(|| {
                        RuntimeError::Internal("snapshot export has no project identity".into())
                    })?;
                    runtime_snapshot::encode(&RuntimeSnapshotPayload {
                        format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
                        artifact_id: vm.artifact_id(),
                        project_identity: project.project_identity,
                        resource_count: u64::try_from(project.resources.len()).unwrap_or(u64::MAX),
                        resource_graph: project.resource_graph.clone(),
                        epoch: self.epoch.0,
                        vm_snapshot,
                        presentation: self.presentation.clone(),
                        operations: self.operations.clone(),
                        controller: self.controller.clone(),
                        logical_time_ns: self.logical_time_ns,
                        random_seed: self.random_seed,
                        selected_locale: self.selected_locale.clone(),
                        culture_table_version: CULTURE_TABLE_VERSION,
                        message_skip: self.message_skip,
                        skip_print: self.skip_print,
                        user_defined_skip: self.user_defined_skip,
                        saved_skip: self.saved_skip,
                        command_intents: self.command_intents.clone(),
                        reusable_system_intents: self.reusable_system_intents.clone(),
                        save_extensions: self.save_extensions.clone(),
                        system_menu: match self.system_menu {
                            SystemMenuState::Title => 0,
                            SystemMenuState::LoadSlots => 1,
                            SystemMenuState::SaveSlots => 2,
                            SystemMenuState::ConfirmOverwrite { .. } => 3,
                        },
                        system_menu_slot: match self.system_menu {
                            SystemMenuState::ConfirmOverwrite { slot } => Some(slot),
                            _ => None,
                        },
                        load_slot_paths: self.load_slot_paths.clone(),
                        occupied_slot_paths: self.occupied_slot_paths.clone(),
                        system_menu_host_request: self.system_menu_host_request,
                        system_menu_page: self.system_menu_page,
                    })
                    .map_err(RuntimeError::Internal)?
                }
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "state export exceeds the negotiated transfer limit",
                );
            }
            let export_artifact_id = (request.kind == StateExportKind::VmSnapshot)
                .then(|| ProtocolBytes::new(vm.artifact_id().bytes()));
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: export_artifact_id,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes,
                next_offset: 0,
            });
            StateExportResult::Ready {
                transfer: descriptor,
            }
        } else {
            StateExportResult::Ineligible { reasons }
        };
        self.emit(
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: request.kind,
                result,
            }),
            Some(message_id),
        )
    }

    fn begin_state_import(
        &mut self,
        message_id: u64,
        request: StateImportBegin,
    ) -> Result<(), RuntimeError> {
        if self.inbound_transfer.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "another state import is already active",
            );
        }
        if request.total_bytes > self.options.limits.maximum_transfer_bytes {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import exceeds the negotiated transfer limit",
            );
        }
        if request.digest.as_slice().len() != blake3::OUT_LEN {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import digest must contain 32 bytes",
            );
        }
        match usize::try_from(request.total_bytes) {
            Ok(_) => {}
            Err(_) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "state import length is not addressable on this platform",
                );
            }
        }
        let transfer_id = self.allocate_transfer();
        self.inbound_transfer = Some(InboundStateTransfer {
            descriptor: StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: request.total_bytes,
                digest: request.digest,
                artifact_id: request.artifact_id,
            },
            // Grow with accepted chunks instead of trusting a potentially huge declaration.
            bytes: Vec::new(),
            committed: false,
        });
        self.emit(
            RuntimeMessage::StateImportAccepted(StateImportAccepted { transfer_id }),
            Some(message_id),
        )
    }

    fn append_state_import(
        &mut self,
        message_id: u64,
        chunk: &StateImportChunk,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state import is active",
            );
        };
        if transfer.descriptor.transfer_id != chunk.transfer_id || transfer.committed {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state import transfer is stale",
            );
        }
        if chunk.offset != u64::try_from(transfer.bytes.len()).unwrap_or(u64::MAX) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import chunks must be contiguous and ordered",
            );
        }
        if chunk.data.as_slice().is_empty()
            || chunk
                .offset
                .saturating_add(u64::try_from(chunk.data.as_slice().len()).unwrap_or(u64::MAX))
                > transfer.descriptor.total_bytes
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import chunk has an invalid length",
            );
        }
        transfer
            .bytes
            .try_reserve(chunk.data.as_slice().len())
            .map_err(|_| RuntimeError::ResourceLimit("state import allocation failed"))?;
        transfer.bytes.extend_from_slice(chunk.data.as_slice());
        Ok(())
    }

    fn commit_state_import(
        &mut self,
        message_id: u64,
        commit: StateImportCommit,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state import is active",
            );
        };
        if transfer.descriptor.transfer_id != commit.transfer_id || transfer.committed {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state import transfer is stale",
            );
        }
        if u64::try_from(transfer.bytes.len()).unwrap_or(u64::MAX)
            != transfer.descriptor.total_bytes
            || transfer.descriptor.digest.as_slice() != blake3::hash(&transfer.bytes).as_bytes()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import length or digest does not match its descriptor",
            );
        }
        transfer.committed = true;
        let kind = transfer.descriptor.kind;
        self.emit(
            RuntimeMessage::StateImportReady(StateImportReady {
                transfer_id: commit.transfer_id,
                kind,
            }),
            Some(message_id),
        )
    }

    fn read_state_export(
        &mut self,
        message_id: u64,
        request: StateExportChunkRequest,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.outbound_transfer.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state export is active",
            );
        };
        if transfer.descriptor.transfer_id != request.transfer_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state export transfer is stale",
            );
        }
        if request.offset != transfer.next_offset {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state export chunks must be read contiguously and in order",
            );
        }
        let offset = match usize::try_from(request.offset) {
            Ok(offset) if offset <= transfer.bytes.len() => offset,
            _ => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "state export offset is outside the payload",
                );
            }
        };
        if request.maximum_bytes == 0 {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state export chunk size must be non-zero",
            );
        }
        let protocol_overhead = 1024_u64;
        let negotiated = self
            .options
            .limits
            .maximum_payload_bytes
            .saturating_sub(protocol_overhead);
        let requested = u64::from(request.maximum_bytes).min(negotiated);
        if requested == 0 {
            return self.reject(
                message_id,
                CommandErrorCode::ResourceLimit,
                "negotiated payload limit cannot carry a state chunk",
            );
        }
        let end = offset
            .saturating_add(usize::try_from(requested).unwrap_or(usize::MAX))
            .min(transfer.bytes.len());
        let complete = end == transfer.bytes.len();
        let response = StateExportChunk {
            transfer_id: request.transfer_id,
            offset: request.offset,
            data: ProtocolBytes::new(transfer.bytes[offset..end].to_vec()),
            complete,
        };
        self.emit(RuntimeMessage::StateExportChunk(response), Some(message_id))?;
        if complete {
            self.outbound_transfer = None;
        } else if let Some(transfer) = self.outbound_transfer.as_mut() {
            transfer.next_offset = u64::try_from(end).unwrap_or(u64::MAX);
        }
        Ok(())
    }

    fn cancel_state_transfer(
        &mut self,
        message_id: u64,
        cancel: StateTransferCancel,
    ) -> Result<(), RuntimeError> {
        let inbound = self
            .inbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.transfer_id == cancel.transfer_id);
        let outbound = self
            .outbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.transfer_id == cancel.transfer_id);
        if !inbound && !outbound {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state transfer is stale",
            );
        }
        if inbound {
            self.inbound_transfer = None;
        }
        if outbound {
            self.outbound_transfer = None;
        }
        Ok(())
    }

    fn consume_state_import(
        &mut self,
        message_id: u64,
        transfer_id: u64,
        kind: StateExportKind,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        let valid = self.inbound_transfer.as_ref().is_some_and(|transfer| {
            transfer.descriptor.transfer_id == transfer_id
                && transfer.descriptor.kind == kind
                && transfer.committed
        });
        if !valid {
            self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "start requires a committed state import of the requested kind",
            )?;
            return Ok(None);
        }
        Ok(self.inbound_transfer.take().map(|transfer| transfer.bytes))
    }

    fn traditional_save_format(&self) -> era_runtime_save::SaveFormat {
        match self.project_snapshot.as_ref() {
            Some(snapshot) if snapshot.save_in_binary && snapshot.compress_save => {
                era_runtime_save::SaveFormat::Binary1808Gzip
            }
            Some(snapshot) if snapshot.save_in_binary => era_runtime_save::SaveFormat::Binary1808,
            _ => era_runtime_save::SaveFormat::Text1808,
        }
    }

    fn shutdown(&mut self, message_id: u64) -> Result<(), RuntimeError> {
        if self.operations.has_candidate_write() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "shutdown cannot cancel a candidate save after its atomic write was emitted",
            );
        }
        self.set_phase(RuntimePhase::Stopping)?;
        let cancelled = self
            .operations
            .total_count()
            .saturating_add(self.effect_journal.len());
        let (service_requests, storage_requests) = self.operations.external_requests();
        for request_id in service_requests {
            self.emit(
                RuntimeMessage::CancelExternalRequest(CancelExternalRequest {
                    request_id,
                    kind: ExternalRequestKind::Service,
                }),
                None,
            )?;
        }
        for request_id in storage_requests {
            self.emit(
                RuntimeMessage::CancelExternalRequest(CancelExternalRequest {
                    request_id,
                    kind: ExternalRequestKind::Storage,
                }),
                None,
            )?;
        }
        self.operations.clear();
        self.effect_journal.clear();
        self.inbound_transfer = None;
        self.outbound_transfer = None;
        self.vm = None;
        self.set_phase(RuntimePhase::Stopped)?;
        self.emit(
            RuntimeMessage::ShutdownReady(ShutdownReady {
                final_runtime_revision: self.revision,
                pending_operations_cancelled: u32::try_from(cancelled).unwrap_or(u32::MAX),
            }),
            Some(message_id),
        )
    }

    fn resynchronize(&mut self, message_id: u64) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::RuntimeResynchronized(RuntimeResynchronized {
                epoch: self.epoch.0,
                phase: self.phase,
                runtime_revision: self.revision,
                presentation: self.presentation.snapshot(),
                exit_requested: self.exit_requested,
                selected_locale: self.selected_locale.clone(),
            }),
            Some(message_id),
        )?;
        if !self.effect_journal.is_empty() {
            self.emit(
                RuntimeMessage::EffectBatch(EffectBatch {
                    effects: self.effect_journal.values().cloned().collect(),
                }),
                Some(message_id),
            )?;
        }
        Ok(())
    }

    fn emit_presentation(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::PresentationSnapshot(self.presentation.snapshot()),
            None,
        )
    }

    fn sync_resource_replay(&mut self) {
        let replay = self
            .project_snapshot
            .as_ref()
            .map(|project| project.resource_graph.replay())
            .unwrap_or_default();
        self.presentation.set_resource_replay(replay);
    }

    fn complete_graphics_result(
        &mut self,
        vm: &mut RuntimeVm,
        request: erabasic_vm::HostRequestId,
        value: i64,
    ) -> Result<(), RuntimeError> {
        commit_integer_result(vm, request, value)?;
        self.sync_resource_replay();
        self.emit_presentation()
    }

    fn set_phase(&mut self, phase: RuntimePhase) -> Result<(), RuntimeError> {
        self.phase = phase;
        self.revision = self.revision.saturating_add(1);
        self.emit(
            RuntimeMessage::StateChanged(RuntimeStateChanged {
                phase,
                revision: self.revision,
                epoch: self.epoch.0,
            }),
            None,
        )
    }

    fn reject(
        &mut self,
        correlation_id: u64,
        code: CommandErrorCode,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::CommandRejected(CommandRejected {
                code,
                message: message.into(),
                recoverable: true,
                source: None,
            }),
            (correlation_id != 0).then_some(correlation_id),
        )
    }

    fn fault(
        &mut self,
        code: FaultCode,
        message: &str,
        origin: Option<erabasic_vm::VmExecutionOrigin>,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Fault(RuntimeFault {
                code,
                message: message.into(),
                origin: origin.map(protocol_execution_origin),
            }),
            None,
        )?;
        self.set_phase(RuntimePhase::Faulted)
    }

    // Taking ownership prevents callers from accidentally retaining a message they
    // believe has been queued, even though encoding itself only borrows it.
    #[allow(clippy::needless_pass_by_value)]
    fn emit(
        &mut self,
        message: RuntimeMessage,
        correlation_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let envelope = message.envelope(
            Some(self.options.session_id),
            Some(self.epoch),
            self.outbound_sequence,
            self.next_message_id,
            correlation_id,
        )?;
        let bytes = encode_envelope(&envelope, self.options.wire_limits)?;
        if self.outbound_journal.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("outbound journal is full"));
        }
        self.outbound.push_back(bytes.clone());
        self.outbound_journal.insert(self.outbound_sequence, bytes);
        self.outbound_sequence = self.outbound_sequence.saturating_add(1);
        self.next_message_id = self.next_message_id.saturating_add(1);
        Ok(())
    }

    fn allocate_request(&mut self) -> Result<u64, RuntimeError> {
        if self.operations.total_count() >= self.options.limits.maximum_pending_requests as usize {
            return Err(RuntimeError::ResourceLimit(
                "too many pending service requests",
            ));
        }
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        Ok(id)
    }

    fn allocate_wait(&mut self) -> u64 {
        let id = self.next_wait_id;
        self.next_wait_id = self.next_wait_id.saturating_add(1);
        id
    }

    fn allocate_transfer(&mut self) -> u64 {
        let id = self.next_transfer_id;
        self.next_transfer_id = self.next_transfer_id.saturating_add(1);
        id
    }

    fn emit_effect(&mut self, kind: EffectKind) -> Result<(), RuntimeError> {
        if self.effect_journal.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("effect journal is full"));
        }
        let event = EffectEvent {
            effect_id: self.next_effect_id,
            kind,
        };
        self.next_effect_id = self.next_effect_id.saturating_add(1);
        self.effect_journal.insert(event.effect_id, event.clone());
        self.emit(
            RuntimeMessage::EffectBatch(EffectBatch {
                effects: vec![event],
            }),
            None,
        )
    }

    fn emit_audio_unavailable(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.audio_device_unavailable".into(),
                severity: DiagnosticSeverity::Warning,
                message: "audio intent was retained but no frontend audio device is available"
                    .into(),
                source: None,
            }),
            None,
        )
    }

    fn acknowledge_effects(
        &mut self,
        message_id: u64,
        acknowledgement: EffectAcknowledgement,
    ) -> Result<(), RuntimeError> {
        let mut seen = BTreeSet::new();
        for outcome in &acknowledgement.outcomes {
            if !seen.insert(outcome.effect_id)
                || !self.effect_journal.contains_key(&outcome.effect_id)
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "effect acknowledgement refers to an unknown or duplicate effect",
                );
            }
        }
        for outcome in acknowledgement.outcomes {
            self.effect_journal.remove(&outcome.effect_id);
            if outcome.status != EffectOutcomeStatus::Completed {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.device_effect_failed".into(),
                        severity: DiagnosticSeverity::Warning,
                        message: outcome.message.unwrap_or_else(|| {
                            format!(
                                "frontend reported {:?} for effect {}",
                                outcome.status, outcome.effect_id
                            )
                        }),
                        source: None,
                    }),
                    Some(message_id),
                )?;
            }
        }
        Ok(())
    }

    fn observe_frontend_time(&mut self, sample: u64) -> u64 {
        let (frontend_origin, logical_origin) = *self
            .frontend_time_origin
            .get_or_insert((sample, self.logical_time_ns));
        let mapped = logical_origin.saturating_add(sample.saturating_sub(frontend_origin));
        self.logical_time_ns = self.logical_time_ns.max(mapped);
        self.logical_time_ns
    }

    fn allocate_interaction(&mut self) -> InteractionToken {
        let token = InteractionToken {
            epoch: self.epoch.0,
            id: self.next_interaction_id,
        };
        self.next_interaction_id = self.next_interaction_id.saturating_add(1);
        token
    }

    fn advance_epoch(&mut self) {
        self.epoch.0 = self.epoch.0.saturating_add(1);
        self.operations.bind_epoch(self.epoch.0);
        self.command_intents.clear();
        self.reusable_system_intents.clear();
        self.next_interaction_id = 1;
        self.accepted_message_ids.clear();
        self.accepted_debug_message_ids.clear();
    }
}

fn runtime_variable_key(
    vm: &RuntimeVm,
    name: &str,
) -> Result<erabasic_bytecode::SymbolKey, RuntimeError> {
    vm.vm()
        .artifact()
        .globals
        .iter()
        .find(|global| global.name.eq_ignore_ascii_case(name))
        .map(|global| global.key)
        .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
}

fn integer_argument_value(arguments: &[VmValue], index: usize) -> Result<i64, RuntimeError> {
    match arguments.get(index) {
        Some(VmValue::Integer(value)) => Ok(*value),
        _ => Err(RuntimeError::Internal(format!(
            "host argument {} must be integer",
            index + 1
        ))),
    }
}

fn i32_argument_value(arguments: &[VmValue], index: usize) -> Result<i32, RuntimeError> {
    i32::try_from(integer_argument_value(arguments, index)?).map_err(|_| {
        RuntimeError::Internal(format!(
            "host argument {} must fit a signed 32-bit drawing coordinate",
            index + 1
        ))
    })
}

fn integer_value_or_zero(value: &VmValue) -> i64 {
    match value {
        VmValue::Integer(value) => *value,
        _ => 0,
    }
}

fn string_argument_value<'a>(
    arguments: &'a [VmValue],
    index: usize,
    command: &str,
) -> Result<&'a str, RuntimeError> {
    match arguments.get(index) {
        Some(VmValue::String(value)) => Ok(value),
        _ => Err(RuntimeError::Internal(format!(
            "{command} argument {} must be string",
            index + 1
        ))),
    }
}

fn save_slot_argument(
    arguments: &[VmValue],
    index: usize,
    command: &str,
) -> Result<u32, RuntimeError> {
    let value = integer_argument_value(arguments, index)?;
    u32::try_from(value)
        .ok()
        .filter(|value| *value <= i32::MAX.cast_unsigned())
        .ok_or_else(|| {
            RuntimeError::Internal(format!(
                "{command} argument {} must be between 0 and {}",
                index + 1,
                i32::MAX
            ))
        })
}

fn save_slot_path(slot: u32) -> String {
    format!("save{slot:02}.sav")
}

fn parse_save_slot(path: &str) -> Option<u32> {
    path.strip_prefix("save")?
        .strip_suffix(".sav")?
        .parse()
        .ok()
}

fn dat_filename(value: &str) -> Result<&str, RuntimeError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains(['/', '\\', '\0'])
        || value.chars().any(char::is_control)
    {
        return Err(RuntimeError::Internal(
            "DAT name must be one safe relative filename component".into(),
        ));
    }
    Ok(value)
}

fn protocol_execution_origin(
    origin: erabasic_vm::VmExecutionOrigin,
) -> era_runtime_protocol::ExecutionOrigin {
    era_runtime_protocol::ExecutionOrigin {
        command: origin.command,
        function: origin.function_name,
        generation: origin.generation.0,
        instruction: origin.instruction,
        source: origin
            .source
            .map(|source| era_runtime_protocol::SourceLocation {
                relative_path: source.relative_path,
                byte_start: source.byte_start,
                byte_end: source.byte_end,
                line: Some(source.line),
                byte_column: Some(source.byte_column),
            }),
    }
}

fn safe_relative_path(value: &str) -> Result<String, RuntimeError> {
    era_runtime_protocol::validate_relative_path(value)
        .map_err(|error| RuntimeError::Internal(error.message))
}

fn safe_relative_directory(value: &str) -> Result<String, RuntimeError> {
    if value.is_empty() || value == "." {
        Ok(String::new())
    } else {
        safe_relative_path(value)
    }
}

fn text_storage_target(value: &VmValue) -> Result<(StorageNamespace, String), RuntimeError> {
    match value {
        VmValue::Integer(value) => {
            let index = u32::try_from(*value)
                .ok()
                .filter(|value| *value <= i32::MAX.cast_unsigned())
                .ok_or_else(|| {
                    RuntimeError::Internal(
                        "text file number must be between 0 and 2147483647".into(),
                    )
                })?;
            Ok((StorageNamespace::Save, format!("txt{index:02}.txt")))
        }
        VmValue::String(value) => {
            let mut path = safe_relative_path(value)?;
            if !path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.contains('.'))
            {
                path.push_str(".txt");
            }
            Ok((StorageNamespace::Data, path))
        }
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => Err(RuntimeError::Internal(
            "text file target must be an integer or string".into(),
        )),
    }
}

fn read_runtime_integer(
    vm: &RuntimeVm,
    name: &str,
    indices: &[u64],
    character: Option<u64>,
) -> Result<i64, RuntimeError> {
    let values = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: runtime_variable_key(vm, name)?,
            indices: indices.to_vec(),
            character,
        }])
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    match values.as_slice() {
        [VmValue::Integer(value)] => Ok(*value),
        _ => Err(RuntimeError::Internal(format!(
            "system variable {name} is not integer"
        ))),
    }
}

fn read_runtime_string(vm: &RuntimeVm, name: &str) -> Result<String, RuntimeError> {
    let values = vm
        .read_runtime_state(&[erabasic_vm::VmRuntimeRead {
            variable: runtime_variable_key(vm, name)?,
            indices: Vec::new(),
            character: None,
        }])
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    match values.as_slice() {
        [VmValue::String(value)] => Ok(value.clone()),
        _ => Err(RuntimeError::Internal(format!(
            "system variable {name} is not string"
        ))),
    }
}

fn write_runtime_integer(
    vm: &mut RuntimeVm,
    name: &str,
    indices: &[u64],
    character: Option<u64>,
    value: i64,
) -> Result<(), RuntimeError> {
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![VmRuntimeWrite {
                variable: runtime_variable_key(vm, name)?,
                indices: indices.to_vec(),
                character,
                value: VmValue::Integer(value),
            }],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

fn write_runtime_string(vm: &mut RuntimeVm, name: &str, value: String) -> Result<(), RuntimeError> {
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![VmRuntimeWrite {
                variable: runtime_variable_key(vm, name)?,
                indices: Vec::new(),
                character: None,
                value: VmValue::String(value),
            }],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

fn fill_runtime_variable(
    vm: &mut RuntimeVm,
    name: &str,
    value: VmValue,
    all_characters: bool,
) -> Result<(), RuntimeError> {
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: Vec::new(),
            fills: vec![VmRuntimeFill {
                variable: runtime_variable_key(vm, name)?,
                value,
                all_characters,
            }],
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

fn reset_training_state(vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
    let artifact = vm.vm().artifact();
    let key = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case(name))
            .map(|global| global.key)
            .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
    };
    let mut writes = vec![
        VmRuntimeWrite {
            variable: key("ASSIPLAY")?,
            indices: Vec::new(),
            character: None,
            value: VmValue::Integer(0),
        },
        VmRuntimeWrite {
            variable: key("PREVCOM")?,
            indices: Vec::new(),
            character: None,
            value: VmValue::Integer(-1),
        },
        VmRuntimeWrite {
            variable: key("NEXTCOM")?,
            indices: Vec::new(),
            character: None,
            value: VmValue::Integer(-1),
        },
    ];
    let fills = [
        "TFLAG", "TSTR", "GOTJUEL", "TEQUIP", "EX", "PALAM", "SOURCE", "TCVAR",
    ]
    .into_iter()
    .map(|name| {
        Ok(VmRuntimeFill {
            variable: key(name)?,
            value: if name == "TSTR" {
                VmValue::String(String::new())
            } else {
                VmValue::Integer(0)
            },
            all_characters: matches!(
                name,
                "GOTJUEL" | "TEQUIP" | "EX" | "PALAM" | "SOURCE" | "TCVAR"
            ),
        })
    })
    .collect::<Result<Vec<_>, RuntimeError>>()?;
    let character_count = vm.vm().export_era_state().characters.len();
    let stain = key("STAIN")?;
    let stain_defaults = artifact
        .project_data
        .static_data
        .replace
        .stain_default
        .clone();
    for character in 0..character_count {
        for (index, value) in stain_defaults.iter().copied().enumerate() {
            writes.push(VmRuntimeWrite {
                variable: stain,
                indices: vec![u64::try_from(index).unwrap_or(u64::MAX)],
                character: Some(u64::try_from(character).unwrap_or(u64::MAX)),
                value: VmValue::Integer(value),
            });
        }
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes,
            fills,
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

fn reset_after_show_user(vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
    let mut fills = Vec::new();
    for name in ["UP", "DOWN", "LOSEBASE", "DOWNBASE", "CUP", "CDOWN"] {
        fills.push(VmRuntimeFill {
            variable: runtime_variable_key(vm, name)?,
            value: VmValue::Integer(0),
            all_characters: matches!(name, "DOWNBASE" | "CUP" | "CDOWN"),
        });
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: Vec::new(),
            fills,
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PurchaseResult {
    Purchased,
    OutOfStock,
    NotEnoughMoney,
}

#[allow(clippy::too_many_lines)]
fn purchase_item(
    vm: &mut RuntimeVm,
    item: usize,
    maximum_shop_items: u32,
) -> Result<PurchaseResult, RuntimeError> {
    if item >= usize::try_from(maximum_shop_items).unwrap_or(usize::MAX) {
        return Ok(PurchaseResult::OutOfStock);
    }
    let artifact = vm.vm().artifact();
    let item_names = artifact
        .project_data
        .static_data
        .name_tables
        .get(&erabasic_data::NameTableKind::Item);
    let price = artifact
        .project_data
        .static_data
        .item_prices
        .get(item)
        .copied();
    if price.is_none()
        || item_names
            .and_then(|table| table.names.get(item))
            .and_then(Option::as_ref)
            .is_none()
    {
        return Ok(PurchaseResult::OutOfStock);
    }
    let find = |name: &str| {
        artifact
            .globals
            .iter()
            .find(|global| global.name.eq_ignore_ascii_case(name))
            .map(|global| global.key)
            .ok_or_else(|| RuntimeError::Internal(format!("system variable {name} is missing")))
    };
    let sales = find("ITEMSALES")?;
    let money = find("MONEY")?;
    let items = find("ITEM")?;
    let bought = find("BOUGHT")?;
    let values = vm
        .read_runtime_state(&[
            erabasic_vm::VmRuntimeRead {
                variable: sales,
                indices: vec![u64::try_from(item).unwrap_or(u64::MAX)],
                character: None,
            },
            erabasic_vm::VmRuntimeRead {
                variable: money,
                indices: Vec::new(),
                character: None,
            },
            erabasic_vm::VmRuntimeRead {
                variable: items,
                indices: vec![u64::try_from(item).unwrap_or(u64::MAX)],
                character: None,
            },
        ])
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    let [
        VmValue::Integer(for_sale),
        VmValue::Integer(current_money),
        VmValue::Integer(owned),
    ] = values.as_slice()
    else {
        return Err(RuntimeError::Internal(
            "shop variables have incompatible types".into(),
        ));
    };
    if *for_sale == 0 {
        return Ok(PurchaseResult::OutOfStock);
    }
    let price = price.expect("checked above");
    if *current_money < price {
        return Ok(PurchaseResult::NotEnoughMoney);
    }
    let prepared = vm
        .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
            writes: vec![
                VmRuntimeWrite {
                    variable: money,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(current_money - price),
                },
                VmRuntimeWrite {
                    variable: items,
                    indices: vec![u64::try_from(item).unwrap_or(u64::MAX)],
                    character: None,
                    value: VmValue::Integer(owned.saturating_add(1)),
                },
                VmRuntimeWrite {
                    variable: bought,
                    indices: Vec::new(),
                    character: None,
                    value: VmValue::Integer(i64::try_from(item).unwrap_or(i64::MAX)),
                },
            ],
            fills: Vec::new(),
            clear_characters: false,
            add_characters_from_csv: Vec::new(),
        })
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_runtime_state(prepared)
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    Ok(PurchaseResult::Purchased)
}

fn commit_completion(
    vm: &mut RuntimeVm,
    request: erabasic_vm::HostRequestId,
    completion: VmHostCompletion,
) -> Result<(), RuntimeError> {
    let prepared = vm
        .validate_host_completion(request, completion)
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
    vm.commit_host_completion(prepared)
        .map(|_| ())
        .map_err(|error| RuntimeError::Internal(error.to_string()))
}

fn commit_integer_result(
    vm: &mut RuntimeVm,
    request: erabasic_vm::HostRequestId,
    value: i64,
) -> Result<(), RuntimeError> {
    commit_completion(
        vm,
        request,
        VmHostCompletion::Ready(HostReady {
            value: Some(VmValue::Integer(value)),
            writes: Vec::new(),
        }),
    )
}

fn commit_host_result_write(
    vm: &mut RuntimeVm,
    request: erabasic_vm::HostRequestId,
    value: i64,
) -> Result<(), RuntimeError> {
    let writes = global_place(vm, "RESULT")
        .map(|target| {
            vec![HostWrite {
                target,
                value: VmValue::Integer(value),
            }]
        })
        .unwrap_or_default();
    commit_completion(
        vm,
        request,
        VmHostCompletion::Ready(HostReady {
            value: None,
            writes,
        }),
    )
}

fn global_place(vm: &RuntimeVm, name: &str) -> Option<PlaceDescriptor> {
    vm.vm()
        .artifact()
        .globals
        .iter()
        .find(|global| global.name.eq_ignore_ascii_case(name))
        .map(|global| PlaceDescriptor {
            variable: global.key,
            indices: vec![0; global.dimensions.len()],
            character: None,
            fiber: None,
            frame: None,
        })
}

fn global_place_at(vm: &RuntimeVm, name: &str, index: usize) -> Option<PlaceDescriptor> {
    let mut place = global_place(vm, name)?;
    let first = place.indices.first_mut()?;
    *first = u64::try_from(index).ok()?;
    Some(place)
}

fn enum_name_matches(operation: &str, candidate: &str, query: &str) -> bool {
    let candidate = candidate.to_uppercase();
    let query = query.to_uppercase();
    if operation.ends_with("BEGINSWITH") {
        candidate.starts_with(&query)
    } else if operation.ends_with("ENDSWITH") {
        candidate.ends_with(&query)
    } else {
        candidate.contains(&query)
    }
}

fn string_array_writes(
    vm: &RuntimeVm,
    target: Option<PlaceDescriptor>,
    values: &[String],
) -> Vec<HostWrite> {
    let Some(base) = target.or_else(|| global_place_at(vm, "RESULTS", 0)) else {
        return Vec::new();
    };
    let maximum = vm
        .vm()
        .artifact()
        .globals
        .iter()
        .find(|definition| definition.key == base.variable)
        .and_then(|definition| definition.dimensions.first())
        .and_then(|value| usize::try_from(*value).ok())
        .unwrap_or(0);
    values
        .iter()
        .take(maximum)
        .enumerate()
        .map(|(index, value)| {
            let mut target = base.clone();
            if let Some(last) = target.indices.last_mut() {
                *last = u64::try_from(index).unwrap_or(u64::MAX);
            } else {
                target
                    .indices
                    .push(u64::try_from(index).unwrap_or(u64::MAX));
            }
            HostWrite {
                target,
                value: VmValue::String(value.clone()),
            }
        })
        .collect()
}

fn is_print(name: &str) -> bool {
    name.starts_with("PRINT") || name == "REUSELASTLINE"
}

fn is_input_command(name: &str) -> bool {
    matches!(
        name,
        "WAIT"
            | "WAITANYKEY"
            | "FORCEWAIT"
            | "TWAIT"
            | "INPUT"
            | "INPUTS"
            | "ONEINPUT"
            | "ONEINPUTS"
            | "TINPUT"
            | "TINPUTS"
            | "TONEINPUT"
            | "TONEINPUTS"
            | "INPUTANY"
            | "BINPUT"
            | "BINPUTS"
            | "ONEBINPUT"
            | "ONEBINPUTS"
            | "INPUTMOUSEKEY"
    )
}

fn is_runtime_print_command(name: &str) -> bool {
    is_print(name)
        || is_input_command(name)
        || matches!(
            name,
            "DRAWLINE"
                | "CLEARLINE"
                | "HTML_PRINT"
                | "HTML_PRINT_ISLAND"
                | "HTML_PRINT_ISLAND_CLEAR"
                | "PRINT_IMG"
                | "PRINT_RECT"
                | "PRINT_SPACE"
        )
}

fn is_column_print(name: &str) -> bool {
    matches!(name, "PRINTC" | "PRINTLC" | "PRINTFORMC" | "PRINTFORMLC")
}

fn print_commits_line(name: &str) -> bool {
    name.ends_with('L') || name.ends_with('W')
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum InputSubmission {
    Value(VmValue),
    Primitive(PrimitiveResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PrimitiveResult {
    fields: [i32; 5],
    selection: Option<VmValue>,
}

fn input_value(
    pending: &PendingInput,
    token: InteractionToken,
    intent: InputIntent,
) -> Option<InputSubmission> {
    if let InputIntent::Activate(activated) = intent {
        return (token == pending.wait.submission_token)
            .then(|| {
                pending
                    .choices
                    .get(&activated)
                    .cloned()
                    .map(InputSubmission::Value)
            })
            .flatten();
    }
    if token != pending.wait.submission_token {
        return None;
    }
    match (pending.wait.kind, intent) {
        (WaitKind::EnterKey | WaitKind::AnyKey, InputIntent::Continue)
        | (WaitKind::EnterKey, InputIntent::Enter)
        | (WaitKind::AnyKey, InputIntent::AnyKey(_)) => {
            Some(InputSubmission::Value(VmValue::Integer(0)))
        }
        (WaitKind::IntegerValue, InputIntent::CommitText(value)) => value
            .parse()
            .ok()
            .map(VmValue::Integer)
            .map(InputSubmission::Value),
        (WaitKind::StringValue, InputIntent::CommitText(value)) => {
            Some(InputSubmission::Value(VmValue::String(value)))
        }
        (WaitKind::AnyValue, InputIntent::CommitText(value)) => Some(InputSubmission::Value(
            value
                .parse()
                .map_or_else(|_| VmValue::String(value), VmValue::Integer),
        )),
        (WaitKind::PrimitiveMouseKey, InputIntent::Primitive(value))
            if matches!(value.input_type, 1..=3) =>
        {
            let selection = match value.selection_token {
                Some(token) => Some(pending.choices.get(&token)?.clone()),
                None => None,
            };
            Some(InputSubmission::Primitive(PrimitiveResult {
                fields: [
                    value.input_type,
                    value.result_1,
                    value.result_2,
                    value.result_3,
                    value.result_4,
                ],
                selection,
            }))
        }
        _ => None,
    }
}

fn selected_capabilities(client: &ClientCapabilities) -> ClientCapabilities {
    ClientCapabilities {
        input_modalities: client.input_modalities.clone(),
        rich_text: client.rich_text,
        html: client.html,
        graphics: client.graphics,
        audio: client.audio,
        // Video and frontend-dependent font metrics still require typed services.
        video: false,
        font_metrics: false,
        column_cells: client.column_cells,
        separators: client.separators,
        available_fonts: {
            let mut fonts = client.available_fonts.clone();
            fonts.sort_by_key(|name| name.to_lowercase());
            fonts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
            fonts
        },
        services: selected_service_capabilities(&client.services),
        storage: client.storage,
    }
}

fn selected_service_capabilities(client: &[ServiceCapability]) -> Vec<ServiceCapability> {
    let mut selected = client
        .iter()
        .filter_map(|capability| {
            let supported = match (capability.kind, capability.operation.as_str()) {
                (ServiceKind::Clock, LOCAL_DATE_TIME_OPERATION) => {
                    LOCAL_DATE_TIME_OPERATION_VERSION
                }
                (ServiceKind::Entropy, RANDOM_SEED_OPERATION) => RANDOM_SEED_OPERATION_VERSION,
                (ServiceKind::InputState, GET_KEY_STATE_OPERATION) => {
                    GET_KEY_STATE_OPERATION_VERSION
                }
                (ServiceKind::Image, IMAGE_METADATA_OPERATION) => IMAGE_METADATA_OPERATION_VERSION,
                (ServiceKind::Image, IMAGE_PIXEL_OPERATION) => IMAGE_PIXEL_OPERATION_VERSION,
                (ServiceKind::Network, UPDATE_CHECK_OPERATION) => UPDATE_CHECK_OPERATION_VERSION,
                (ServiceKind::OpenUrl, OPEN_URL_OPERATION) => OPEN_URL_OPERATION_VERSION,
                _ => return None,
            };
            negotiate_version(capability.versions, VersionRange::exact(supported)).map(|version| {
                ServiceCapability {
                    kind: capability.kind,
                    operation: capability.operation.clone(),
                    versions: VersionRange::exact(version),
                }
            })
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        (left.kind, left.operation.as_str()).cmp(&(right.kind, right.operation.as_str()))
    });
    selected.dedup_by(|left, right| left.kind == right.kind && left.operation == right.operation);
    selected
}

fn select_locale(preferred: &[String]) -> &'static str {
    for locale in preferred {
        let locale = locale.to_ascii_lowercase();
        if locale == "zh-hans" || locale.starts_with("zh-cn") || locale.starts_with("zh-sg") {
            return "zh-Hans";
        }
        if locale == "en" || locale.starts_with("en-") {
            return "en";
        }
        if locale == "ja" || locale.starts_with("ja-") {
            return "ja";
        }
    }
    "ja"
}

fn localized_system_text(locale: &str, key: SystemTextKey) -> String {
    let value = match (locale, key) {
        ("zh-Hans", SystemTextKey::InvalidValue) => "输入无效",
        ("zh-Hans", SystemTextKey::SaveQuestion) => "请选择保存位置",
        ("zh-Hans", SystemTextKey::LoadQuestion) => "请选择要读取的存档",
        ("zh-Hans", SystemTextKey::OverwriteQuestion) => "要覆盖这个存档吗？",
        ("zh-Hans", SystemTextKey::NotEnoughMoney) => "金钱不足",
        ("zh-Hans", SystemTextKey::OutOfStock) => "无法购买",
        ("zh-Hans", SystemTextKey::AutoSaveFailed) => "自动保存失败",
        ("zh-Hans", SystemTextKey::AutoSaveSkipped) => "已跳过自动保存",
        ("zh-Hans", SystemTextKey::PressAnyKey) => "请按任意键",
        ("zh-Hans", SystemTextKey::SaveSlot) => "存档",
        ("zh-Hans", SystemTextKey::Back) => "返回",
        ("zh-Hans", SystemTextKey::NewGame) => "开始新游戏",
        ("zh-Hans", SystemTextKey::LoadGame) => "读取存档",
        ("en", SystemTextKey::InvalidValue) => "Invalid value",
        ("en", SystemTextKey::SaveQuestion) => "Select a save slot",
        ("en", SystemTextKey::LoadQuestion) => "Select a save to load",
        ("en", SystemTextKey::OverwriteQuestion) => "Overwrite this save?",
        ("en", SystemTextKey::NotEnoughMoney) => "Not enough money",
        ("en", SystemTextKey::OutOfStock) => "This item cannot be purchased",
        ("en", SystemTextKey::AutoSaveFailed) => "Autosave failed",
        ("en", SystemTextKey::AutoSaveSkipped) => "Autosave skipped",
        ("en", SystemTextKey::PressAnyKey) => "Press any key",
        ("en", SystemTextKey::SaveSlot) => "Save",
        ("en", SystemTextKey::Back) => "Back",
        ("en", SystemTextKey::NewGame) => "Start a new game",
        ("en", SystemTextKey::LoadGame) => "Load game",
        (_, SystemTextKey::InvalidValue) => "入力が正しくありません",
        (_, SystemTextKey::SaveQuestion) => "セーブするデータを選択してください",
        (_, SystemTextKey::LoadQuestion) => "ロードするデータを選択してください",
        (_, SystemTextKey::OverwriteQuestion) => "上書きしてよろしいですか？",
        (_, SystemTextKey::NotEnoughMoney) => "所持金が足りません",
        (_, SystemTextKey::OutOfStock) => "購入できません",
        (_, SystemTextKey::AutoSaveFailed) => "オートセーブに失敗しました",
        (_, SystemTextKey::AutoSaveSkipped) => "オートセーブをスキップしました",
        (_, SystemTextKey::PressAnyKey) => "何かキーを押してください",
        (_, SystemTextKey::SaveSlot) => "セーブデータ",
        (_, SystemTextKey::Back) => "戻る",
        (_, SystemTextKey::NewGame) => "最初からはじめる",
        (_, SystemTextKey::LoadGame) => "ロードする",
    };
    value.into()
}

fn protocol_to_vm(value: &era_runtime_protocol::ProtocolValue) -> VmValue {
    match value {
        era_runtime_protocol::ProtocolValue::Integer(value) => VmValue::Integer(*value),
        era_runtime_protocol::ProtocolValue::String(value) => VmValue::String(value.clone()),
        era_runtime_protocol::ProtocolValue::Boolean(value) => VmValue::Integer(i64::from(*value)),
        era_runtime_protocol::ProtocolValue::Bytes(_) => VmValue::String(String::new()),
    }
}

fn calendar_number(time: LocalDateTimeResponse) -> i64 {
    let date = i64::from(time.year) * 10_000_000_000
        + i64::from(time.month) * 100_000_000
        + i64::from(time.day) * 1_000_000
        + i64::from(time.hour) * 10_000
        + i64::from(time.minute) * 100
        + i64::from(time.second);
    date * 1000 + i64::from(time.millisecond)
}

fn complete_frozen_clock(
    vm: &mut RuntimeVm,
    request: &VmHostRequest,
    time: LocalDateTimeResponse,
) -> Result<(), RuntimeError> {
    let name = request.import.import.name.to_ascii_uppercase();
    let operation = match name.as_str() {
        "GETTIME" => ClockOperation::Time,
        "GETTIMES" => ClockOperation::Times,
        "GETMILLISECOND" => ClockOperation::Millisecond,
        "GETSECOND" => ClockOperation::Second,
        _ => {
            return Err(RuntimeError::Internal(format!(
                "clock operation {name} has no frozen candidate implementation"
            )));
        }
    };
    let mut writes = Vec::new();
    let value = if request.import.import.result.is_none() {
        if let Some(target) = global_place(vm, "RESULT") {
            writes.push(HostWrite {
                target,
                value: VmValue::Integer(calendar_number(time)),
            });
        }
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(calendar_string(time)),
            });
        }
        None
    } else {
        Some(match operation {
            ClockOperation::Time => VmValue::Integer(calendar_number(time)),
            ClockOperation::Times => VmValue::String(calendar_string(time)),
            ClockOperation::Millisecond => VmValue::Integer(milliseconds_since_year_one(time)),
            ClockOperation::Second => VmValue::Integer(milliseconds_since_year_one(time) / 1_000),
        })
    };
    commit_completion(
        vm,
        request.id,
        VmHostCompletion::Ready(HostReady { value, writes }),
    )
}

fn calendar_string(time: LocalDateTimeResponse) -> String {
    format!(
        "{:04}/{:02}/{:02} {:02}:{:02}:{:02}",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

fn milliseconds_since_year_one(time: LocalDateTimeResponse) -> i64 {
    const DAYS_BEFORE_MONTH: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    // This is the proleptic Gregorian calendar used by DateTime.Now.Ticks.
    let year_before = i64::from(time.year) - 1;
    let days_before_year =
        year_before * 365 + year_before / 4 - year_before / 100 + year_before / 400;
    let mut days = days_before_year
        + DAYS_BEFORE_MONTH[usize::from(time.month.saturating_sub(1).min(11))]
        + i64::from(time.day.saturating_sub(1));
    if time.month > 2 && is_leap_year(time.year) {
        days += 1;
    }
    (((days * 24 + i64::from(time.hour)) * 60 + i64::from(time.minute)) * 60
        + i64::from(time.second))
        * 1000
        + i64::from(time.millisecond)
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn intersect_limits(left: RuntimeLimits, right: RuntimeLimits) -> RuntimeLimits {
    RuntimeLimits {
        maximum_envelope_bytes: left
            .maximum_envelope_bytes
            .min(right.maximum_envelope_bytes),
        maximum_payload_bytes: left.maximum_payload_bytes.min(right.maximum_payload_bytes),
        maximum_pending_requests: left
            .maximum_pending_requests
            .min(right.maximum_pending_requests),
        maximum_journal_entries: left
            .maximum_journal_entries
            .min(right.maximum_journal_entries),
        maximum_drive_instructions: left
            .maximum_drive_instructions
            .min(right.maximum_drive_instructions),
        maximum_transfer_bytes: left
            .maximum_transfer_bytes
            .min(right.maximum_transfer_bytes),
    }
}

fn debugger_suspends_message(message: &RuntimeMessage) -> bool {
    matches!(
        message,
        RuntimeMessage::ProjectManifest(_)
            | RuntimeMessage::Start(_)
            | RuntimeMessage::Input(_)
            | RuntimeMessage::ServiceResponse(_)
            | RuntimeMessage::StorageResponse(_)
            | RuntimeMessage::StateExportRequest(_)
            | RuntimeMessage::StateImportBegin(_)
            | RuntimeMessage::StateImportChunk(_)
            | RuntimeMessage::StateImportCommit(_)
            | RuntimeMessage::StateExportChunkRequest(_)
            | RuntimeMessage::StateTransferCancel(_)
            | RuntimeMessage::ReloadProject(_)
    )
}

fn format_era_integer(value: i64, format: &str) -> Result<String, &'static str> {
    if format.is_empty() {
        return Ok(value.to_string());
    }
    let mut chars = format.chars();
    let first = chars.next().expect("non-empty format");
    let precision = chars.as_str().parse::<usize>().ok();
    match first.to_ascii_uppercase() {
        'D' if chars.as_str().is_empty() || precision.is_some() => {
            let width = precision.unwrap_or(0);
            let magnitude = value.unsigned_abs().to_string();
            let digits = format!("{magnitude:0>width$}");
            Ok(if value < 0 {
                format!("-{digits}")
            } else {
                digits
            })
        }
        'X' if chars.as_str().is_empty() || precision.is_some() => {
            let width = precision.unwrap_or(0);
            if first.is_ascii_lowercase() {
                Ok(format!("{value:0>width$x}"))
            } else {
                Ok(format!("{value:0>width$X}"))
            }
        }
        'N' if chars.as_str().is_empty() || precision.is_some() => {
            let decimals = precision.unwrap_or(2);
            let grouped = group_decimal(value);
            Ok(if decimals == 0 {
                grouped
            } else {
                format!("{grouped}.{}", "0".repeat(decimals))
            })
        }
        _ if format
            .chars()
            .all(|character| matches!(character, '#' | '0' | ','))
            && format.contains('0') =>
        {
            let minimum = format.chars().filter(|character| *character == '0').count();
            let magnitude = value.unsigned_abs().to_string();
            let mut digits = format!("{magnitude:0>minimum$}");
            if format.contains(',') {
                digits = group_unsigned_decimal(&digits);
            }
            Ok(if value < 0 {
                format!("-{digits}")
            } else {
                digits
            })
        }
        _ => Err("unsupported integer format"),
    }
}

fn group_decimal(value: i64) -> String {
    let digits = group_unsigned_decimal(&value.unsigned_abs().to_string());
    if value < 0 {
        format!("-{digits}")
    } else {
        digits
    }
}

fn group_unsigned_decimal(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + value.len() / 3);
    for (index, character) in value.chars().enumerate() {
        if index != 0 && (value.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(character);
    }
    output
}

// Version 1 of the deterministic width table covers the ASCII block and the
// half-width katakana block used by Emuera projects.  It deliberately avoids
// the platform-dependent VisualBasic StrConv implementation.
const HALF_KANA: &str = "｡｢｣､･ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝﾞﾟ";
const FULL_KANA: &str = "。「」、・ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン゛゜";

fn to_full_width(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut input = value.chars().peekable();
    while let Some(character) = input.next() {
        if let Some(mark) = input.peek().copied()
            && matches!(mark, 'ﾞ' | 'ﾟ')
            && let Some(composed) = compose_half_kana(character, mark)
        {
            output.push(composed);
            input.next();
            continue;
        }
        match character {
            ' ' => output.push('　'),
            '!'..='~' => output.push(char::from_u32(u32::from(character) + 0xfee0).unwrap()),
            _ => output.push(map_width_char(character, HALF_KANA, FULL_KANA).unwrap_or(character)),
        }
    }
    output
}

fn to_half_width(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if let Some(pair) = decompose_full_kana(character) {
            output.extend(pair);
            continue;
        }
        match character {
            '　' => output.push(' '),
            '\u{ff01}'..='\u{ff5e}' => {
                output.push(char::from_u32(u32::from(character) - 0xfee0).unwrap());
            }
            _ => output.push(map_width_char(character, FULL_KANA, HALF_KANA).unwrap_or(character)),
        }
    }
    output
}

fn map_width_char(character: char, source: &str, target: &str) -> Option<char> {
    source
        .chars()
        .position(|candidate| candidate == character)
        .and_then(|index| target.chars().nth(index))
}

fn compose_half_kana(base: char, mark: char) -> Option<char> {
    let bases = "ｳｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎﾊﾋﾌﾍﾎ";
    let marks = "ﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾟﾟﾟﾟﾟ";
    let full = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポ";
    bases
        .chars()
        .zip(marks.chars())
        .position(|(candidate, candidate_mark)| candidate == base && candidate_mark == mark)
        .and_then(|index| full.chars().nth(index))
}

fn decompose_full_kana(character: char) -> Option<[char; 2]> {
    let full = "ヴガギグゲゴザジズゼゾダヂヅデドバビブベボパピプペポ";
    let bases = "ｳｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾊﾋﾌﾍﾎﾊﾋﾌﾍﾎ";
    let marks = "ﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾞﾟﾟﾟﾟﾟ";
    full.chars()
        .position(|candidate| candidate == character)
        .and_then(|index| Some([bases.chars().nth(index)?, marks.chars().nth(index)?]))
}

#[cfg(test)]
mod tests {
    use era_debug_protocol::{DEBUG_PROTOCOL_VERSION, DebugHello, DebugMessage, DebugScope};
    use era_protocol::{Channel, Envelope, ProtocolBytes, decode_envelope, encode_envelope};
    use era_runtime_protocol::{
        FileCategory, FileChange, FilePayload, ProjectManifest, SubmittedFile,
    };

    use super::*;

    fn capabilities() -> ClientCapabilities {
        ClientCapabilities {
            input_modalities: vec![era_runtime_protocol::InputModality::Keyboard],
            rich_text: false,
            html: false,
            graphics: false,
            audio: false,
            video: false,
            font_metrics: false,
            column_cells: true,
            separators: true,
            available_fonts: vec!["sans-serif".into()],
            services: vec![
                ServiceCapability {
                    kind: ServiceKind::Clock,
                    operation: LOCAL_DATE_TIME_OPERATION.into(),
                    versions: VersionRange::exact(LOCAL_DATE_TIME_OPERATION_VERSION),
                },
                ServiceCapability {
                    kind: ServiceKind::Entropy,
                    operation: RANDOM_SEED_OPERATION.into(),
                    versions: VersionRange::exact(RANDOM_SEED_OPERATION_VERSION),
                },
                ServiceCapability {
                    kind: ServiceKind::InputState,
                    operation: GET_KEY_STATE_OPERATION.into(),
                    versions: VersionRange::exact(GET_KEY_STATE_OPERATION_VERSION),
                },
            ],
            storage: StorageCapabilities {
                revisions: true,
                atomic_replace: true,
                missing_precondition: true,
                delete: true,
            },
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn submit(session: &mut RuntimeSession, sequence: u64, message: RuntimeMessage) {
        let mut envelope = Envelope::new(
            Channel::Runtime,
            RUNTIME_PROTOCOL_VERSION,
            sequence,
            sequence + 1,
            message.tag(),
            ProtocolBytes::new(message.encode_payload().expect("encode message")),
        );
        if sequence != 0 {
            envelope.session = Some(session.options.session_id);
            envelope.session_epoch = Some(session.epoch);
        }
        let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
        session.submit_envelope(&bytes).expect("submit envelope");
    }

    fn drain(session: &mut RuntimeSession) -> Vec<RuntimeMessage> {
        let mut messages = Vec::new();
        while let Some(bytes) = session.poll_envelope() {
            let envelope = decode_envelope(&bytes, WireLimits::default()).expect("decode envelope");
            messages.push(RuntimeMessage::from_envelope(&envelope).expect("decode message"));
        }
        messages
    }

    fn submit_debug(session: &mut RuntimeSession, sequence: u64, message: &DebugMessage) {
        let envelope = message
            .envelope(
                Some(session.options.session_id),
                Some(session.epoch),
                sequence,
                10_000 + sequence,
                None,
            )
            .expect("debug envelope");
        let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode debug");
        session.submit_envelope(&bytes).expect("submit debug");
    }

    #[test]
    fn handshake_selects_only_implemented_features() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: vec![RuntimeFeature::Audio, RuntimeFeature::TimedInput],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("drive");
        let messages = drain(&mut session);
        let RuntimeMessage::ServerHello(hello) = &messages[0] else {
            panic!("expected server hello");
        };
        assert_eq!(hello.selected_version, RUNTIME_PROTOCOL_VERSION);
        assert!(hello.features.contains(&RuntimeFeature::TimedInput));
        assert!(!hello.features.contains(&RuntimeFeature::Audio));
        assert_eq!(hello.selected_capabilities.storage, capabilities().storage);
    }

    #[test]
    fn debug_channel_has_independent_sequence_and_cannot_widen_creator_policy() {
        let mut session = RuntimeSession::new(RuntimeOptions {
            debug_scope_mask: (1 << 2) | (1 << 5),
            ..RuntimeOptions::default()
        });
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "debug-test".into(),
                features: Vec::new(),
                capabilities: capabilities(),
                requested_limits: RuntimeOptions::default().limits,
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let _ = drain(&mut session);

        submit_debug(
            &mut session,
            0,
            &DebugMessage::Hello(DebugHello {
                versions: VersionRange::exact(DEBUG_PROTOCOL_VERSION),
                requested_scopes: vec![
                    DebugScope::ExecutionControl,
                    DebugScope::VariablesWrite,
                    DebugScope::GameFieldsRead,
                ],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let bytes = session.poll_envelope().expect("debug grant");
        let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
        let DebugMessage::Grant(grant) = DebugMessage::from_envelope(&envelope).unwrap() else {
            panic!("expected debug grant");
        };
        assert_eq!(
            grant.scopes,
            vec![DebugScope::GameFieldsRead, DebugScope::ExecutionControl]
        );
        assert_eq!(grant.token.session_epoch, session.epoch.0);
    }

    #[test]
    fn debugger_pause_freezes_frontend_time_until_resume() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.state = SessionState::Active;
        session.phase = RuntimePhase::DebugPaused;
        session.logical_time_ns = 500;
        session.frontend_time_origin = Some((10, 500));
        session
            .handle_message(
                1,
                RuntimeMessage::AdvanceTime(AdvanceTime {
                    monotonic_time_ns: 1_000,
                }),
            )
            .unwrap();
        assert_eq!(session.logical_time_ns, 500);
        session.resume_debug_time();
        assert_eq!(session.frontend_time_origin, Some((1_000, 500)));
    }

    #[test]
    fn ready_project_reload_stages_and_commits_a_normalized_delta() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "reload-test".into(),
                features: vec![RuntimeFeature::ProjectReload],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::ReloadProject(ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: vec![FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "./main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(
                            "@SYSTEM_TITLE\nPRINTL reloaded\nRETURN\n".into(),
                        ),
                        content_hash: None,
                    },
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        assert_eq!(session.phase(), RuntimePhase::Ready);
        assert_eq!(
            session
                .project_snapshot
                .as_ref()
                .unwrap()
                .manifest
                .project_revision,
            2
        );
        assert!(messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(era_runtime_protocol::ProjectLoadReport {
                project_revision: 2,
                success: true,
                ..
            })
        )));
    }

    #[test]
    fn state_import_rejects_out_of_order_chunks_and_bad_digests() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: vec![RuntimeFeature::TraditionalSave],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);

        submit(
            &mut session,
            1,
            RuntimeMessage::StateImportBegin(StateImportBegin {
                kind: StateExportKind::TraditionalSave,
                total_bytes: 3,
                digest: ProtocolBytes::new([0; 32]),
                artifact_id: None,
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let transfer_id = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
                _ => None,
            })
            .unwrap();

        submit(
            &mut session,
            2,
            RuntimeMessage::StateImportChunk(StateImportChunk {
                transfer_id,
                offset: 1,
                data: ProtocolBytes::new([b'a']),
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(CommandRejected {
                code: CommandErrorCode::InvalidValue,
                ..
            })
        )));

        submit(
            &mut session,
            3,
            RuntimeMessage::StateImportChunk(StateImportChunk {
                transfer_id,
                offset: 0,
                data: ProtocolBytes::new(*b"abc"),
            }),
        );
        submit(
            &mut session,
            4,
            RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(CommandRejected {
                code: CommandErrorCode::InvalidValue,
                ..
            })
        )));
    }

    #[test]
    fn training_reset_updates_shared_and_all_character_state_atomically() {
        let build = build_project(
            &ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@EVENTTRAIN\nRETURN\n".into()),
                    content_hash: None,
                }],
            },
            None,
        );
        let artifact = build.artifact.expect("valid project");
        let source = artifact
            .artifact()
            .globals
            .iter()
            .find(|global| global.name == "SOURCE")
            .expect("SOURCE")
            .key;
        let tflag = artifact
            .artifact()
            .globals
            .iter()
            .find(|global| global.name == "TFLAG")
            .expect("TFLAG")
            .key;
        let mut vm = RuntimeVm::new(artifact, VmConfig::default());
        let dirty = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
                writes: vec![
                    VmRuntimeWrite {
                        variable: source,
                        indices: vec![0],
                        character: Some(0),
                        value: VmValue::Integer(9),
                    },
                    VmRuntimeWrite {
                        variable: tflag,
                        indices: vec![0],
                        character: None,
                        value: VmValue::Integer(7),
                    },
                ],
                fills: Vec::new(),
                clear_characters: false,
                add_characters_from_csv: Vec::new(),
            })
            .expect("prepare dirty state");
        vm.commit_runtime_state(dirty).expect("commit dirty state");
        reset_training_state(&mut vm).expect("reset training state");
        assert_eq!(
            vm.read_runtime_state(&[
                erabasic_vm::VmRuntimeRead {
                    variable: source,
                    indices: vec![0],
                    character: Some(0),
                },
                erabasic_vm::VmRuntimeRead {
                    variable: tflag,
                    indices: vec![0],
                    character: None,
                },
            ]),
            Ok(vec![VmValue::Integer(0), VmValue::Integer(0)])
        );
    }

    #[test]
    fn shop_purchase_validates_stock_and_commits_money_item_and_bought_together() {
        let build = build_project(
            &ProjectManifest {
                project_revision: 1,
                files: vec![
                    SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "ITEM.CSV".into(),
                        category: FileCategory::Csv,
                        payload: FilePayload::Utf8("5,potion,120\n".into()),
                        content_hash: None,
                    },
                ],
            },
            None,
        );
        let artifact = build.artifact.expect("valid project");
        let mut vm = RuntimeVm::new(artifact, VmConfig::default());
        let sales = runtime_variable_key(&vm, "ITEMSALES").unwrap();
        let money = runtime_variable_key(&vm, "MONEY").unwrap();
        let dirty = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::Mutate {
                writes: vec![
                    VmRuntimeWrite {
                        variable: sales,
                        indices: vec![5],
                        character: None,
                        value: VmValue::Integer(1),
                    },
                    VmRuntimeWrite {
                        variable: money,
                        indices: Vec::new(),
                        character: None,
                        value: VmValue::Integer(200),
                    },
                ],
                fills: Vec::new(),
                clear_characters: false,
                add_characters_from_csv: Vec::new(),
            })
            .unwrap();
        vm.commit_runtime_state(dirty).unwrap();
        assert_eq!(
            purchase_item(&mut vm, 5, 100).unwrap(),
            PurchaseResult::Purchased
        );
        assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
        assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
        assert_eq!(read_runtime_integer(&vm, "BOUGHT", &[], None).unwrap(), 5);
        assert_eq!(
            purchase_item(&mut vm, 5, 100).unwrap(),
            PurchaseResult::NotEnoughMoney
        );
        assert_eq!(read_runtime_integer(&vm, "MONEY", &[], None).unwrap(), 80);
        assert_eq!(read_runtime_integer(&vm, "ITEM", &[5], None).unwrap(), 1);
    }

    #[test]
    fn train_controller_consumes_runtime_button_intent_and_loops_after_eventcomend() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        let source = "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN\n@SHOW_USERCOM\nRETURN\n@EVENTCOM\nRETURN\n@COM0\nFLAG:0 += 1\nRESULT = 1\nRETURN\n@SOURCE_CHECK\nRETURN\n@EVENTCOMEND\nRETURN\n";
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![
                    SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(source.into()),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "TRAIN.CSV".into(),
                        category: FileCategory::Csv,
                        payload: FilePayload::Utf8("0,go\n".into()),
                        content_hash: None,
                    },
                ],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(3) },
            }),
        );
        for _ in 0..12 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let pending = session
            .operations
            .active_input()
            .expect("training command wait");
        let token = *pending.choices.keys().next().expect("PRINTBUTTON token");
        let wait_id = pending.wait.wait_id;
        let submission_token = pending.wait.submission_token;
        submit(
            &mut session,
            3,
            RuntimeMessage::Input(FrontendInput {
                wait_id,
                token: submission_token,
                intent: InputIntent::Activate(token),
                monotonic_time_ns: 0,
                message_skip: false,
            }),
        );
        for _ in 0..64 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some()
                && session.controller.step == SystemStep::TrainShowUser
            {
                break;
            }
        }
        let output = drain(&mut session);
        assert_ne!(session.phase, RuntimePhase::Faulted, "{output:#?}");
        let vm = session.vm.as_ref().expect("running VM");
        assert_eq!(read_runtime_integer(vm, "FLAG", &[0], None).unwrap(), 1);
        assert_eq!(
            read_runtime_integer(vm, "SOURCE", &[0], Some(0)).unwrap(),
            0
        );
        assert_eq!(
            session.controller.step,
            SystemStep::TrainShowUser,
            "phase={:?}",
            session.phase
        );
    }

    #[test]
    fn project_load_start_and_print_cross_the_message_boundary() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("hello");
        drain(&mut session);

        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL hello\nRETURN\n".into()),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("load");
        let loaded = drain(&mut session);
        assert!(loaded.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(report) if report.success
        )));
        assert_eq!(session.phase(), RuntimePhase::Ready);

        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..4 {
            session.drive(RuntimeDriveBudget::default()).expect("run");
        }
        assert_eq!(session.random_seed(), Some(1));
        let output = drain(&mut session);
        assert!(output.iter().any(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => snapshot.lines.iter().any(|line| {
                line.runs.iter().any(|run| matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("hello")
                ))
            }),
            _ => false,
        }));
    }

    #[test]
    fn runtime_metadata_queries_use_the_active_artifact_and_fiber() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "metadata-test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "metadata.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\n#DIMS VALUES, 3\n#DIMS CHOICES, 5\nCALL SIZE_OF, CHOICES\nPRINTFORML meta={VARSIZE(\"VALUES\")},{EXISTFUNCTION(\"SYSTEM_TITLE\")},{EXISTVAR(\"VALUES\")},%GETDOINGFUNCTION()%,{RESULT},%CHOICES:2%\nPRINTFORML funcs={ENUMFUNCWITH(\"SIZE\", CHOICES)},%CHOICES:0%\nPRINTFORML vars={ENUMVARWITH(\"SAVEDATA_TEXT\", CHOICES)},%CHOICES:0%\nCALL ORACLE_REFLECTION\nPRINTFORML reflection={RESULT:12},{RESULT:13},%RESULTS:8%,%RESULTS:9%\nRETURN\n@SIZE_OF(refChoices)\n#DIMS REF refChoices, 0\nrefChoices:2 = \"bound\"\nRESULT = VARSIZE(\"refChoices\")\nRETURN\n@ORACLE_REFLECTION\n#DIMS NAMES, 4\nRESULT:12 = ENUMFUNCWITH(\"ORACLE_REFLECTION\", NAMES)\nRESULTS:8 = %NAMES:0%\nRESULT:13 = ENUMVARWITH(\"SAVEDATA_TEXT\", NAMES)\nRESULTS:9 = %NAMES:0%\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let loaded = drain(&mut session);
        assert!(
            loaded.iter().any(|message| {
                matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
            }),
            "{loaded:#?}"
        );
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..24 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
        }
        let output = drain(&mut session);
        assert!(
            output.iter().any(|message| match message {
                RuntimeMessage::PresentationSnapshot(snapshot) =>
                    snapshot.lines.iter().any(|line| {
                        line.runs.iter().any(|run| {
                            matches!(
                                run,
                                era_runtime_protocol::DisplayRun::Text { text, .. }
                                    if text.contains("meta=3,1,0,SYSTEM_TITLE,5,bound")
                            )
                        })
                    }),
                _ => false,
            }),
            "{output:#?}"
        );
        let rendered = output
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .flat_map(|snapshot| snapshot.lines.iter())
            .flat_map(|line| line.runs.iter())
            .filter_map(|run| match run {
                era_runtime_protocol::DisplayRun::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(rendered.contains("funcs=1,SIZE_OF"), "{rendered}");
        assert!(rendered.contains("vars=1,SAVEDATA_TEXT"), "{rendered}");
        assert!(
            rendered.contains("reflection=1,1,ORACLE_REFLECTION,SAVEDATA_TEXT"),
            "{rendered}\n{output:#?}"
        );
    }

    #[test]
    fn reference_presentation_fixture_preserves_logical_intent() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("hello");
        drain(&mut session);

        // Keep this body identical to ORACLE_PRESENTATION in the C# fixture so
        // the oracle and Rust tests exercise the same EraBasic commands.
        let source = "@SYSTEM_TITLE\nPRINTBUTTON \"A\", 1\nPRINTBUTTONC \"B\", 2\nPRINTBUTTONLC \"C\", 3\nPRINTL\nDRAWLINE\nNOSKIP\nPRINTL VISIBLE\nENDNOSKIP\nRETURN\n";
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("load");
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).expect("run");
            if session.phase() == RuntimePhase::Ready {
                break;
            }
        }
        let output = drain(&mut session);
        let snapshot = output
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .next_back()
            .expect("presentation snapshot");

        assert!(snapshot.lines.iter().any(|line| {
            line.runs.iter().any(|run| {
                matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Button { runs, .. }
                        if runs.iter().any(|run| matches!(
                            run,
                            era_runtime_protocol::DisplayRun::Text { text, .. } if text == "A"
                        ))
                )
            })
        }));
        assert_eq!(
            snapshot
                .lines
                .iter()
                .flat_map(|line| &line.runs)
                .filter(|run| matches!(run, era_runtime_protocol::DisplayRun::ColumnCell { .. }))
                .count(),
            2
        );
        assert!(snapshot.lines.iter().any(|line| {
            line.runs
                .iter()
                .any(|run| matches!(run, era_runtime_protocol::DisplayRun::Separator { .. }))
        }));
        assert!(snapshot.lines.iter().any(|line| {
            line.runs.iter().any(|run| {
                matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text { text, .. } if text == "VISIBLE"
                )
            })
        }));
    }

    #[test]
    fn typed_input_updates_result_and_sixth_argument_honors_message_skip() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: vec![RuntimeFeature::TimedInput],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("hello");
        drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nTINPUT 1000, 7, 1, \"timeout\", 0, 0\nTINPUT 1000, 9, 1, \"timeout\", 0, 0\nPRINTFORML got={RESULT}\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("load");
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..4 {
            session.drive(RuntimeDriveBudget::default()).expect("wait");
        }
        let opened = drain(&mut session);
        let wait = opened
            .iter()
            .find_map(|message| match message {
                RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait.clone()),
                _ => None,
            })
            .expect("runtime should publish the input wait");
        assert_eq!(
            wait.default_value,
            Some(era_runtime_protocol::ProtocolValue::Integer(7))
        );
        assert_eq!(wait.stability, WaitStability::Transient);

        submit(
            &mut session,
            3,
            RuntimeMessage::Input(FrontendInput {
                wait_id: wait.wait_id,
                token: wait.submission_token,
                monotonic_time_ns: 10,
                intent: InputIntent::CommitText("42".into()),
                message_skip: true,
            }),
        );
        for _ in 0..4 {
            session
                .drive(RuntimeDriveBudget::default())
                .expect("resume");
        }
        let output = drain(&mut session);
        assert!(output.iter().any(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => snapshot.lines.iter().any(|line| {
                line.runs.iter().any(|run| matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("got=9")
                ))
            }),
            _ => false,
        }));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn traditional_save_export_and_restore_are_atomic_runtime_operations() {
        fn prepare() -> RuntimeSession {
            let mut session = RuntimeSession::new(RuntimeOptions::default());
            submit(
                &mut session,
                0,
                RuntimeMessage::ClientHello(ClientHello {
                    runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                    client_name: "save-test".into(),
                    features: vec![RuntimeFeature::TraditionalSave, RuntimeFeature::VmSnapshot],
                    requested_limits: RuntimeOptions::default().limits,
                    capabilities: capabilities(),
                    preferred_locales: vec!["en-US".into()],
                }),
            );
            session.drive(RuntimeDriveBudget::default()).unwrap();
            drain(&mut session);
            submit(
                &mut session,
                1,
                RuntimeMessage::ProjectManifest(ProjectManifest {
                    project_revision: 1,
                    files: vec![
                        SubmittedFile {
                            relative_path: "variables.erh".into(),
                            category: FileCategory::Erh,
                            payload: FilePayload::Utf8("#DIM SAVEDATA ZZZSAVE\n".into()),
                            content_hash: None,
                        },
                        SubmittedFile {
                            relative_path: "save.erb".into(),
                            category: FileCategory::Erb,
                            payload: FilePayload::Utf8(
                                "@SYSTEM_TITLE\nINPUT\nZZZSAVE = RESULT\nINPUT\nRETURN\n@SYSTEM_LOADEND\nPRINTFORML loadend={ZZZSAVE}\nRETURN\n@EVENTLOAD\nPRINTL eventload\nRETURN\n@SHOW_SHOP\nPRINTL shop\nWAIT\nRETURN\n@SAVEINFO\nPRINTL unexpected-autosave\nRETURN\n"
                                    .into(),
                            ),
                            content_hash: None,
                        },
                    ],
                }),
            );
            session.drive(RuntimeDriveBudget::default()).unwrap();
            let load_messages = drain(&mut session);
            assert_eq!(session.phase(), RuntimePhase::Ready, "{load_messages:#?}");
            session
        }

        let mut source = prepare();
        submit(
            &mut source,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..4 {
            source.drive(RuntimeDriveBudget::default()).unwrap();
        }
        let wait = drain(&mut source)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait),
                _ => None,
            })
            .expect("first INPUT wait");
        submit(
            &mut source,
            3,
            RuntimeMessage::Input(FrontendInput {
                wait_id: wait.wait_id,
                token: wait.submission_token,
                monotonic_time_ns: 1,
                intent: InputIntent::CommitText("37".into()),
                message_skip: false,
            }),
        );
        for _ in 0..4 {
            source.drive(RuntimeDriveBudget::default()).unwrap();
        }
        drain(&mut source);
        assert_eq!(source.phase(), RuntimePhase::WaitingInput);
        submit(
            &mut source,
            4,
            RuntimeMessage::StateExportRequest(StateExportRequest {
                kind: StateExportKind::TraditionalSave,
            }),
        );
        source.drive(RuntimeDriveBudget::default()).unwrap();
        let descriptor = drain(&mut source)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateExportReady(StateExportReady {
                    result: StateExportResult::Ready { transfer },
                    ..
                }) => Some(transfer),
                _ => None,
            })
            .expect("traditional save descriptor");
        submit(
            &mut source,
            5,
            RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
                transfer_id: descriptor.transfer_id,
                offset: 0,
                maximum_bytes: u32::MAX,
            }),
        );
        source.drive(RuntimeDriveBudget::default()).unwrap();
        let bytes = drain(&mut source)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateExportChunk(chunk) => Some(chunk.data.as_slice().to_vec()),
                _ => None,
            })
            .expect("traditional save bytes");

        let mut restored = prepare();
        submit(
            &mut restored,
            2,
            RuntimeMessage::StateImportBegin(StateImportBegin {
                kind: StateExportKind::TraditionalSave,
                total_bytes: u64::try_from(bytes.len()).unwrap(),
                digest: descriptor.digest,
                artifact_id: None,
            }),
        );
        restored.drive(RuntimeDriveBudget::default()).unwrap();
        let transfer_id = drain(&mut restored)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
                _ => None,
            })
            .expect("accepted import");
        submit(
            &mut restored,
            3,
            RuntimeMessage::StateImportChunk(StateImportChunk {
                transfer_id,
                offset: 0,
                data: ProtocolBytes::new(bytes),
            }),
        );
        submit(
            &mut restored,
            4,
            RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
        );
        restored.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut restored);
        submit(
            &mut restored,
            5,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::TraditionalSave { transfer_id },
            }),
        );
        for _ in 0..5 {
            restored.drive(RuntimeDriveBudget::default()).unwrap();
        }
        let output = drain(&mut restored);
        let display = output
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::PresentationSnapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .flat_map(|snapshot| &snapshot.lines)
            .flat_map(|line| &line.runs)
            .filter_map(|run| match run {
                era_runtime_protocol::DisplayRun::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("|");
        let loadend = display.find("loadend=37").expect("SYSTEM_LOADEND output");
        let eventload = display.find("eventload").expect("EVENTLOAD output");
        let shop = display.find("shop").expect("SHOW_SHOP output");
        assert!(loadend < eventload && eventload < shop, "{display}");
        assert!(!display.contains("unexpected-autosave"), "{display}");

        let old_wait = source
            .operations
            .active_input()
            .expect("snapshot wait")
            .wait
            .clone();
        submit(
            &mut source,
            6,
            RuntimeMessage::StateExportRequest(StateExportRequest {
                kind: StateExportKind::VmSnapshot,
            }),
        );
        source.drive(RuntimeDriveBudget::default()).unwrap();
        let snapshot_descriptor = drain(&mut source)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateExportReady(StateExportReady {
                    result: StateExportResult::Ready { transfer },
                    ..
                }) => Some(transfer),
                _ => None,
            })
            .expect("runtime snapshot descriptor");
        let mut snapshot_bytes = Vec::new();
        let mut source_sequence = 7;
        loop {
            submit(
                &mut source,
                source_sequence,
                RuntimeMessage::StateExportChunkRequest(StateExportChunkRequest {
                    transfer_id: snapshot_descriptor.transfer_id,
                    offset: u64::try_from(snapshot_bytes.len()).unwrap(),
                    maximum_bytes: 1024 * 1024,
                }),
            );
            source_sequence += 1;
            source.drive(RuntimeDriveBudget::default()).unwrap();
            let chunk = drain(&mut source)
                .into_iter()
                .find_map(|message| match message {
                    RuntimeMessage::StateExportChunk(chunk) => Some(chunk),
                    _ => None,
                })
                .expect("runtime snapshot chunk");
            snapshot_bytes.extend_from_slice(chunk.data.as_slice());
            if chunk.complete {
                break;
            }
        }

        let mut exact = prepare();
        submit(
            &mut exact,
            2,
            RuntimeMessage::StateImportBegin(StateImportBegin {
                kind: StateExportKind::VmSnapshot,
                total_bytes: u64::try_from(snapshot_bytes.len()).unwrap(),
                digest: snapshot_descriptor.digest,
                artifact_id: snapshot_descriptor.artifact_id,
            }),
        );
        exact.drive(RuntimeDriveBudget::default()).unwrap();
        let transfer_id = drain(&mut exact)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
                _ => None,
            })
            .unwrap();
        let mut exact_sequence = 3;
        for (index, chunk) in snapshot_bytes.chunks(1024 * 1024).enumerate() {
            submit(
                &mut exact,
                exact_sequence,
                RuntimeMessage::StateImportChunk(StateImportChunk {
                    transfer_id,
                    offset: u64::try_from(index * 1024 * 1024).unwrap(),
                    data: ProtocolBytes::new(chunk.to_vec()),
                }),
            );
            exact_sequence += 1;
        }
        submit(
            &mut exact,
            exact_sequence,
            RuntimeMessage::StateImportCommit(StateImportCommit { transfer_id }),
        );
        exact_sequence += 1;
        exact.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut exact);
        submit(
            &mut exact,
            exact_sequence,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::VmSnapshot { transfer_id },
            }),
        );
        exact.drive(RuntimeDriveBudget::default()).unwrap();
        let restored_wait = exact.operations.active_input().expect("restored wait");
        assert_eq!(exact.phase(), RuntimePhase::WaitingInput);
        assert_ne!(restored_wait.wait.wait_id, old_wait.wait_id);
        assert_ne!(
            restored_wait.wait.submission_token,
            old_wait.submission_token
        );
        assert_eq!(restored_wait.wait.submission_token.epoch, exact.epoch.0);
    }

    #[test]
    fn empty_storage_listing_opens_a_fixed_runtime_tokenized_page() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.state = SessionState::Active;
        session.phase = RuntimePhase::WaitingExternal;
        session.epoch = SessionEpoch(1);
        session.selected_locale = "en".into();
        session.storage_capabilities = StorageCapabilities {
            revisions: true,
            atomic_replace: true,
            missing_precondition: true,
            delete: true,
        };
        session
            .operations
            .insert_storage(7, PendingStorage::ListLoadSlots);
        session
            .complete_storage(
                10,
                StorageResponse {
                    request_id: 7,
                    result: StorageResult::Listed {
                        entries: Vec::new(),
                    },
                },
            )
            .unwrap();
        assert_eq!(
            session.load_slot_paths.first().map(String::as_str),
            Some("save00.sav")
        );
        assert_eq!(
            session.load_slot_paths.last().map(String::as_str),
            Some("save99.sav")
        );
        assert_eq!(session.load_slot_paths.len(), 21);
        assert!(session.occupied_slot_paths.is_empty());
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        let wait = session.operations.active_input().expect("system slot wait");
        assert!(wait.wait.system_input);
        assert!(
            wait.choices
                .keys()
                .all(|token| token.epoch == session.epoch.0)
        );
        assert!(session.presentation.snapshot().lines.iter().any(|line| {
            line.runs.iter().any(|run| {
                matches!(
                    run,
                    era_runtime_protocol::DisplayRun::Text {
                        system_text: Some(reference),
                        ..
                    } if reference.key == SystemTextKey::LoadQuestion
                )
            })
        }));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn nested_savegame_cancel_resumes_the_suspended_vm_call() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "save-menu-test".into(),
                features: vec![RuntimeFeature::Storage, RuntimeFeature::VmSnapshot],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "menu.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SHOW_SHOP\nSAVEGAME\nRESULT = 7\nWAIT\nRETURN\n".into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        let artifact = session.artifact.clone().expect("compiled menu fixture");
        let entry = artifact
            .artifact()
            .functions
            .iter()
            .find(|function| function.name == "SHOW_SHOP")
            .expect("SHOW_SHOP")
            .key;
        let code = artifact
            .artifact()
            .functions
            .iter()
            .find(|function| function.key == entry)
            .unwrap()
            .code
            .clone();
        let mut vm = RuntimeVm::new(artifact, VmConfig::default());
        vm.spawn_entry(entry, Vec::new()).unwrap();
        session.vm = Some(vm);
        session.controller.flow = Some(SystemFlow::Normal);
        session.phase = RuntimePhase::Running;

        let mut request = None;
        let mut observed = Vec::new();
        let mut reports = Vec::new();
        for _ in 0..4 {
            reports.push(session.drive(RuntimeDriveBudget::default()).unwrap());
            let messages = drain(&mut session);
            request = messages.iter().find_map(|message| match message {
                RuntimeMessage::StorageRequest(request) => Some(request.clone()),
                _ => None,
            });
            observed.extend(messages);
            if request.is_some() {
                break;
            }
        }
        let request = request.unwrap_or_else(|| {
            panic!(
                "SAVEGAME list request; phase={:?}, code={code:#?}, reports={reports:#?}, output={observed:#?}",
                session.phase,
            )
        });
        assert!(matches!(request.operation, StorageOperation::List { .. }));
        session
            .complete_storage(
                2,
                StorageResponse {
                    request_id: request.request_id,
                    result: StorageResult::Listed {
                        entries: vec![StorageEntry {
                            relative_path: "save01.sav".into(),
                            byte_length: 3,
                            revision: Some("r1".into()),
                        }],
                    },
                },
            )
            .unwrap();
        let scan = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StorageRequest(request) => Some(request),
                _ => None,
            })
            .expect("slot metadata read");
        assert_eq!(scan.relative_path, "save01.sav");
        session
            .complete_storage(
                3,
                StorageResponse {
                    request_id: scan.request_id,
                    result: StorageResult::Read {
                        data: ProtocolBytes::new(b"bad".to_vec()),
                        revision: Some("r1".into()),
                    },
                },
            )
            .unwrap();
        assert!(session.invalid_slot_paths.contains("save01.sav"));
        let pending = session
            .operations
            .take_active_input()
            .expect("save menu wait");
        assert!(pending.host_request.is_some());
        session.operations.restore_active_input(pending.clone());
        assert!(session.operations.is_snapshot_stable());
        assert!(session.vm.as_ref().unwrap().snapshot().is_ok());
        session
            .export_state(
                99,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                },
            )
            .unwrap();
        assert!(drain(&mut session).into_iter().any(|message| matches!(
            message,
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { .. },
                ..
            })
        )));
        let pending = session.operations.take_active_input().unwrap();
        assert!(
            pending
                .choices
                .values()
                .any(|value| value == &VmValue::Integer(-1_001))
        );
        session
            .finish_system_input(pending, &VmValue::Integer(-1_001))
            .unwrap();
        let delete = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StorageRequest(request) => Some(request),
                _ => None,
            })
            .expect("revision-bound slot delete");
        assert_eq!(delete.relative_path, "save01.sav");
        assert!(matches!(
            delete.operation,
            StorageOperation::Delete {
                precondition: StoragePrecondition::Revision(ref revision),
            } if revision == "r1"
        ));
        session
            .complete_storage(
                4,
                StorageResponse {
                    request_id: delete.request_id,
                    result: StorageResult::Error {
                        error: era_runtime_protocol::FrontendIoError {
                            kind: FrontendIoErrorKind::Conflict,
                            message: "changed".into(),
                            platform_code: None,
                        },
                    },
                },
            )
            .unwrap();
        assert!(session.operations.active_input().is_some());
        let pending = session.operations.take_active_input().unwrap();
        session
            .finish_system_input(pending, &VmValue::Integer(-1))
            .unwrap();
        for _ in 0..4 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
            7
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn savedata_uses_atomic_frontend_storage_and_resumes_only_after_completion() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "storage-test".into(),
                features: vec![RuntimeFeature::Storage],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "save.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nPUTFORM suffix\nRESULT = SAVENOS()\nSAVEDATA 2, \"slot\"\nWAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        let mut request = None;
        for _ in 0..8 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            for message in drain(&mut session) {
                if let RuntimeMessage::StorageRequest(value) = message {
                    request = Some(value);
                }
            }
            if request.is_some() {
                break;
            }
        }
        let request = request.expect("SAVEDATA storage request");
        assert_eq!(request.namespace, StorageNamespace::Save);
        assert_eq!(request.relative_path, "save02.sav");
        let StorageOperation::Write {
            data,
            atomic_replace,
            precondition,
        } = request.operation
        else {
            panic!("SAVEDATA must write")
        };
        assert!(atomic_replace);
        assert_eq!(precondition, StoragePrecondition::Any);
        let decoded = era_runtime_save::decode(
            data.as_slice(),
            era_runtime_save::SaveCodecLimits::default(),
        )
        .expect("current save bytes");
        assert_eq!(decoded.metadata.description, "slot");
        assert_eq!(session.phase(), RuntimePhase::WaitingExternal);

        submit(
            &mut session,
            3,
            RuntimeMessage::StorageResponse(StorageResponse {
                request_id: request.request_id,
                result: StorageResult::Written {
                    revision: Some("r1".into()),
                },
            }),
        );
        for _ in 0..8 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        assert_eq!(session.phase(), RuntimePhase::WaitingInput);
        assert_eq!(
            session
                .operations
                .active_input()
                .expect("WAIT after save")
                .wait
                .kind,
            WaitKind::EnterKey
        );
        let vm = session.vm.as_ref().expect("runtime VM");
        assert_eq!(read_runtime_string(vm, "SAVEDATA_TEXT").unwrap(), "suffix");
        assert_eq!(read_runtime_integer(vm, "RESULT", &[], None).unwrap(), 20);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn saveinfo_candidate_is_isolated_until_the_storage_commit() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "candidate-test".into(),
                features: vec![RuntimeFeature::Storage],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![SubmittedFile {
                    relative_path: "candidate.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nWAIT\nRETURN\n@SAVEINFO\nRESULT = 99\nRESULT:1 = GETCONFIG(\"Font size\")\nRESULTS:1 = %BARSTR(2, 4, 4)%\nPUTFORM suffix\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..4 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
        }
        drain(&mut session);
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
            0
        );

        let time = LocalDateTimeResponse {
            year: 2026,
            month: 7,
            day: 17,
            hour: 12,
            minute: 34,
            second: 56,
            millisecond: 0,
            utc_offset_minutes: 480,
        };
        let mut live = session.vm.take().unwrap();
        session
            .begin_candidate_save(&mut live, 99, CandidateSaveContinuation::Autosave)
            .unwrap();
        session.vm = Some(live);
        let stat_request = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StorageRequest(request) => Some(request),
                _ => None,
            })
            .unwrap();
        assert_eq!(stat_request.operation, StorageOperation::Stat);
        session
            .complete_storage(
                0,
                StorageResponse {
                    request_id: stat_request.request_id,
                    result: StorageResult::Metadata(era_runtime_protocol::StorageMetadata {
                        byte_length: 12,
                        revision: Some("slot-rev".into()),
                    }),
                },
            )
            .unwrap();
        let clock_request = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request) => Some(request),
                _ => None,
            })
            .unwrap();
        session
            .complete_service(
                0,
                ServiceResponse {
                    request_id: clock_request.request_id,
                    result: ServiceResult::Ready {
                        payload: ProtocolBytes::new(encode_canonical(&time).unwrap()),
                    },
                },
            )
            .unwrap();
        let write_request = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::StorageRequest(request) => Some(request),
                _ => None,
            })
            .unwrap();
        let StorageOperation::Write {
            data: bytes,
            atomic_replace,
            precondition,
        } = write_request.operation
        else {
            panic!("candidate did not issue a write")
        };
        assert!(atomic_replace);
        assert_eq!(
            precondition,
            StoragePrecondition::Revision("slot-rev".into())
        );
        let decoded = decode_scoped_save(
            bytes.as_slice(),
            session.vm.as_ref().unwrap().vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
        )
        .unwrap();
        assert_eq!(decoded.description, "2026/07/17 12:34:56 suffix");
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
            0,
            "candidate mutation leaked before commit"
        );
        session
            .complete_storage(
                0,
                StorageResponse {
                    request_id: write_request.request_id,
                    result: StorageResult::Written {
                        revision: Some("new-rev".into()),
                    },
                },
            )
            .unwrap();
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
            99
        );
        assert_eq!(
            read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
            18
        );
        let vm = session.vm.as_ref().unwrap();
        let results = runtime_variable_key(vm, "RESULTS").unwrap();
        assert_eq!(
            vm.vm().read_variable(results, &[1], None),
            Ok(VmValue::String("[**..]".into()))
        );
    }

    #[test]
    fn sequence_gaps_are_rejected_before_execution() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        let message = RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
        });
        let envelope = message
            .envelope(None, None, 2, 1, None)
            .expect("create envelope");
        let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
        assert!(matches!(
            session.submit_envelope(&bytes),
            Err(RuntimeError::InvalidSequence {
                expected: 0,
                actual: 2
            })
        ));
    }

    #[test]
    fn active_session_rejects_stale_epochs_and_acknowledges_journal_entries() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "test".into(),
                features: vec![RuntimeFeature::StateResynchronization],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["ja".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).expect("hello");
        drain(&mut session);
        assert_eq!(session.outbound_journal.len(), 1);

        let ack = RuntimeMessage::Acknowledge(era_runtime_protocol::SequenceAcknowledgement {
            through_sequence: 0,
        });
        submit(&mut session, 1, ack);
        session.drive(RuntimeDriveBudget::default()).expect("ack");
        assert!(session.outbound_journal.is_empty());

        let message = RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 1,
        });
        let mut envelope = message
            .envelope(
                Some(session.options.session_id),
                Some(SessionEpoch(session.epoch.0.saturating_sub(1))),
                2,
                3,
                None,
            )
            .expect("stale envelope");
        envelope.session = Some(session.options.session_id);
        let bytes = encode_envelope(&envelope, WireLimits::default()).expect("encode envelope");
        assert!(matches!(
            session.submit_envelope(&bytes),
            Err(RuntimeError::SessionMismatch)
        ));
    }

    #[test]
    fn configuration_is_parsed_and_resources_receive_stable_identities() {
        let build = build_project(
            &ProjectManifest {
                project_revision: 1,
                files: vec![
                    SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "emuera.config".into(),
                        category: FileCategory::Configuration,
                        payload: FilePayload::Utf8("Language=Chinese".into()),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "resources.csv".into(),
                        category: FileCategory::ResourceManifest,
                        payload: FilePayload::Utf8("; name,path".into()),
                        content_hash: None,
                    },
                ],
            },
            None,
        );
        assert!(build.report.success);
        let codes: Vec<_> = build
            .report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect();
        assert!(codes.contains(&"runtime.invalid_configuration"));
        assert!(!codes.contains(&"runtime.resource_manifest_deferred"));
        let snapshot = build.snapshot.expect("normalized project snapshot");
        assert_eq!(snapshot.resources.len(), 1);
        assert_eq!(snapshot.resources[0].relative_path, "resources.csv");
        assert_eq!(
            snapshot.resources[0].payload_digest,
            *blake3::hash(b"; name,path").as_bytes()
        );
        assert_ne!(snapshot.project_identity, [0; 32]);
    }

    #[test]
    fn frontend_calendar_values_match_dotnet_datetime_shapes() {
        let time = LocalDateTimeResponse {
            year: 2026,
            month: 7,
            day: 15,
            hour: 13,
            minute: 4,
            second: 5,
            millisecond: 6,
            utc_offset_minutes: 480,
        };
        assert_eq!(calendar_number(time), 20_260_715_130_405_006);
        assert_eq!(calendar_string(time), "2026/07/15 13:04:05");
        assert_eq!(
            milliseconds_since_year_one(LocalDateTimeResponse {
                year: 1,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                millisecond: 0,
                utc_offset_minutes: 0,
            }),
            0
        );
        assert_eq!(milliseconds_since_year_one(time) / 1_000, 63_919_717_445);
    }

    #[test]
    fn deterministic_width_and_integer_format_tables_cover_era_usage() {
        assert_eq!(to_full_width("ABC 123 ｶﾞﾊﾟ"), "ＡＢＣ　１２３　ガパ");
        assert_eq!(to_half_width("ＡＢＣ　１２３　ガパ"), "ABC 123 ｶﾞﾊﾟ");
        assert_eq!(format_era_integer(12_345, "#,##0"), Ok("12,345".into()));
        assert_eq!(format_era_integer(-7, "D3"), Ok("-007".into()));
        assert_eq!(format_era_integer(255, "X4"), Ok("00FF".into()));
    }

    #[test]
    fn frontend_monotonic_time_rebases_onto_restored_logical_time() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.logical_time_ns = 100;
        assert_eq!(session.observe_frontend_time(5), 100);
        assert_eq!(session.observe_frontend_time(15), 110);
        assert_eq!(session.observe_frontend_time(9), 110);
    }

    #[test]
    fn primitive_input_uses_runtime_selection_tokens_and_rejects_timeout_spoofing() {
        let submission = InteractionToken { epoch: 7, id: 1 };
        let selection = InteractionToken { epoch: 7, id: 2 };
        let pending = PendingInput {
            host_request: None,
            wait: InputWait {
                wait_id: 9,
                kind: WaitKind::PrimitiveMouseKey,
                stability: WaitStability::Transient,
                one_input: false,
                stop_message_skip: false,
                system_input: false,
                mouse_input: true,
                default_value: None,
                deadline_ns: Some(10),
                display_time: false,
                timeout_message: None,
                submission_token: submission,
                countdown_remaining_ms: None,
            },
            result_name: Some("RESULT".into()),
            choices: BTreeMap::from([(selection, VmValue::Integer(42))]),
            timeout_duration_ns: Some(10),
            post_input: None,
        };
        let input = era_runtime_protocol::PrimitiveInput {
            input_type: 1,
            result_1: 10,
            result_2: 20,
            result_3: 1,
            result_4: 3,
            selection_token: Some(selection),
        };
        assert_eq!(
            input_value(&pending, submission, InputIntent::Primitive(input)),
            Some(InputSubmission::Primitive(PrimitiveResult {
                fields: [1, 10, 20, 1, 3],
                selection: Some(VmValue::Integer(42)),
            }))
        );
        assert_eq!(
            input_value(&pending, submission, InputIntent::Activate(selection)),
            Some(InputSubmission::Value(VmValue::Integer(42)))
        );
        assert!(
            input_value(
                &pending,
                InteractionToken { epoch: 7, id: 99 },
                InputIntent::Activate(selection),
            )
            .is_none()
        );
        assert!(
            input_value(
                &pending,
                submission,
                InputIntent::Primitive(era_runtime_protocol::PrimitiveInput {
                    input_type: 4,
                    result_1: 0,
                    result_2: 0,
                    result_3: 0,
                    result_4: 0,
                    selection_token: None,
                }),
            )
            .is_none()
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn project_resource_metadata_is_frontend_decoded_before_load_commit() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        let mut client = capabilities();
        client.graphics = true;
        client.services.push(ServiceCapability {
            kind: ServiceKind::Image,
            operation: IMAGE_METADATA_OPERATION.into(),
            versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
        });
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "resource-test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: client,
                preferred_locales: vec!["en".into()],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let _ = drain(&mut session);
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                project_revision: 1,
                files: vec![
                    SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "resources/sprites.csv".into(),
                        category: FileCategory::ResourceManifest,
                        payload: FilePayload::Utf8("FACE,face.png".into()),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "resources/face.png".into(),
                        category: FileCategory::Resource,
                        payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                        content_hash: None,
                    },
                ],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::ProjectLoadReport(_)))
        );
        let request_id = messages
            .iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == IMAGE_METADATA_OPERATION =>
                {
                    Some(request.request_id)
                }
                _ => None,
            })
            .expect("image metadata request");
        submit(
            &mut session,
            2,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&ImageMetadataResponse {
                            width: 32,
                            height: 16,
                            format: "png".into(),
                            animated: false,
                        })
                        .unwrap(),
                    ),
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        assert!(drain(&mut session).iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        }));
        assert_eq!(
            session
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite("face"))
                .map(|sprite| (sprite.width, sprite.height)),
            Some((32, 16))
        );

        submit(
            &mut session,
            3,
            RuntimeMessage::ReloadProject(ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: vec![FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "resources/face.png".into(),
                        category: FileCategory::Resource,
                        payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 5, 6])),
                        content_hash: None,
                    },
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let reload_messages = drain(&mut session);
        let reload_request = reload_messages
            .iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == IMAGE_METADATA_OPERATION =>
                {
                    Some(request.request_id)
                }
                _ => None,
            })
            .expect("changed image metadata request");
        assert_eq!(
            session
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite("face"))
                .map(|sprite| (sprite.width, sprite.height)),
            Some((32, 16)),
            "the live graph must not change before candidate metadata commits"
        );
        submit(
            &mut session,
            4,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: reload_request,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&ImageMetadataResponse {
                            width: 64,
                            height: 24,
                            format: "png".into(),
                            animated: false,
                        })
                        .unwrap(),
                    ),
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        assert!(drain(&mut session).iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success && report.project_revision == 2)
        }));
        assert_eq!(
            session
                .project_snapshot
                .as_ref()
                .and_then(|project| project.resource_graph.sprite("face"))
                .map(|sprite| (sprite.width, sprite.height)),
            Some((64, 24))
        );

        submit(
            &mut session,
            5,
            RuntimeMessage::ReloadProject(ReloadProject {
                base_revision: 2,
                target_revision: 3,
                changes: vec![FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "resources/face.png".into(),
                        category: FileCategory::Resource,
                        payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
                        content_hash: None,
                    },
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let failed_request = drain(&mut session)
            .into_iter()
            .find_map(|message| match message {
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == IMAGE_METADATA_OPERATION =>
                {
                    Some(request.request_id)
                }
                _ => None,
            })
            .expect("second changed image metadata request");
        submit(
            &mut session,
            6,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: failed_request,
                result: ServiceResult::Error {
                    error: era_runtime_protocol::ServiceError {
                        code: "decoder.invalid".into(),
                        message: "invalid image".into(),
                    },
                },
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let failed = drain(&mut session);
        assert!(failed.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if !report.success && report.project_revision == 3)
        }));
        assert_eq!(
            session
                .project_snapshot
                .as_ref()
                .map(|project| project.manifest.project_revision),
            Some(2),
            "failed candidate metadata must leave the previous project authoritative"
        );
    }

    #[test]
    fn font_profile_is_session_fixed_case_insensitive_and_deterministic() {
        let mut requested = capabilities();
        requested.available_fonts = vec!["Zeta".into(), "alpha".into(), "ALPHA".into()];
        let selected = selected_capabilities(&requested);
        assert_eq!(selected.available_fonts, vec!["alpha", "Zeta"]);
    }

    #[test]
    fn effect_acknowledgements_are_exact_and_failures_become_diagnostics() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.state = SessionState::Active;
        session.epoch = SessionEpoch(1);
        session
            .emit_effect(EffectKind::StartAnimation("flash".into()))
            .expect("emit effect");
        let _ = drain(&mut session);
        session
            .handle_message(
                10,
                RuntimeMessage::EffectAcknowledgement(EffectAcknowledgement {
                    outcomes: vec![era_runtime_protocol::EffectOutcome {
                        effect_id: 1,
                        status: EffectOutcomeStatus::Failed,
                        message: Some("device unavailable".into()),
                    }],
                }),
            )
            .expect("acknowledge effect");
        assert!(session.effect_journal.is_empty());
        assert!(matches!(
            drain(&mut session).as_slice(),
            [RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })]
                if code == "runtime.device_effect_failed"
        ));
    }
}
