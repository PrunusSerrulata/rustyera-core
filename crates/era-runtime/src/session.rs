use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use era_protocol::{
    ProtocolBytes, ProtocolError, ProtocolVersion, SessionEpoch, SessionId, VersionRange,
    WireLimits, decode_canonical, decode_envelope, encode_canonical, encode_envelope,
    negotiate_version,
};
use era_runtime_protocol::{
    AdvanceTime, CancelExternalRequest, CellAlignment, ClientCapabilities, ClientHello,
    CommandErrorCode, CommandRejected, DiagnosticSeverity, ExitReason, ExitRequested,
    ExternalRequestKind, FaultCode, FrontendInput, FrontendIoErrorKind, GET_KEY_STATE_OPERATION,
    GET_KEY_STATE_OPERATION_VERSION, GetKeyStateRequest, GetKeyStateResponse, InputIntent,
    InputWait, InteractionToken, LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION,
    LineAlignment, LocalDateTimeRequest, LocalDateTimeResponse, ProjectManifest,
    ProtocolDiagnostic, RANDOM_SEED_OPERATION, RANDOM_SEED_OPERATION_VERSION,
    RUNTIME_PROTOCOL_VERSION, RandomSeedRequest, RandomSeedResponse, ReloadProject, RuntimeFault,
    RuntimeFeature, RuntimeLimits, RuntimeMessage, RuntimePhase, RuntimeResynchronized,
    RuntimeStateChanged, ServerHello, ServiceKind, ServiceRequest, ServiceResponse, ServiceResult,
    ShutdownReady, SnapshotIneligibleReason, StartMode, StartRequest, StateExportChunk,
    StateExportChunkRequest, StateExportKind, StateExportReady, StateExportRequest,
    StateExportResult, StateImportAccepted, StateImportBegin, StateImportChunk, StateImportCommit,
    StateImportReady, StateTransferCancel, StateTransferDescriptor, StorageNamespace,
    StorageOperation, StoragePrecondition, StorageRequest, StorageResponse, StorageResult,
    SystemTextArgument, SystemTextKey, VersionRejected, WaitChange, WaitKind, WaitStability,
};
use erabasic_compiler::IncrementalState;
use erabasic_validator::ValidatedArtifact;
use erabasic_vm::{
    EraSaveScope, EraState, HostReady, HostWaitStability, HostWrite, PlaceDescriptor, RunBudget,
    RuntimeVm, StructuredScope, VmConfig, VmDriveMode, VmHostCompletion, VmHostRequest,
    VmPortEvent, VmPortStop, VmRestorePort, VmRuntimeFill, VmRuntimePort, VmRuntimeStatePort,
    VmRuntimeStateTransaction, VmRuntimeWrite, VmSnapshot, VmValue,
};
use serde::{Deserialize, Serialize};

use crate::controller::{SystemController, SystemFlow, SystemStep};
use crate::host::{ClockOperation, ExternalCompletion, PendingInput, input_wait};
use crate::operation::{PendingOperations, PendingService, PendingStorage};
use crate::presentation::{PresentationModel, display_value, logical_line_string};
use crate::project::{NormalizedProjectSnapshot, apply_project_delta, build_project};
use crate::runtime_snapshot::{self, RUNTIME_SNAPSHOT_FORMAT_VERSION, RuntimeSnapshotPayload};
use crate::save_adapter::{
    decode_era_save, decode_scoped_save, encode_era_save, encode_scoped_save,
    merge_opaque_extensions, merge_structured_extensions,
};

#[derive(Clone, Copy, Debug)]
pub struct RuntimeOptions {
    pub session_id: SessionId,
    pub limits: RuntimeLimits,
    pub wire_limits: WireLimits,
    pub vm_config: VmConfig,
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

/// Single-owner runtime actor. Methods only enqueue, drive, and dequeue messages;
/// no frontend code can run inside a VM instruction dispatch.
pub struct RuntimeSession {
    options: RuntimeOptions,
    state: SessionState,
    phase: RuntimePhase,
    revision: u64,
    epoch: SessionEpoch,
    expected_inbound_sequence: u64,
    outbound_sequence: u64,
    next_message_id: u64,
    next_request_id: u64,
    next_wait_id: u64,
    next_interaction_id: u64,
    next_transfer_id: u64,
    logical_time_ns: u64,
    frontend_time_origin: Option<(u64, u64)>,
    random_seed: Option<u64>,
    inbound: VecDeque<(u64, RuntimeMessage)>,
    outbound: VecDeque<Vec<u8>>,
    outbound_journal: BTreeMap<u64, Vec<u8>>,
    accepted_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    artifact: Option<ValidatedArtifact>,
    incremental: IncrementalState,
    vm: Option<RuntimeVm>,
    presentation: PresentationModel,
    operations: PendingOperations,
    key_toggle_state: [u8; 256],
    message_skip: bool,
    command_intents: BTreeMap<InteractionToken, VmValue>,
    reusable_system_intents: BTreeMap<InteractionToken, VmValue>,
    exit_requested: Option<ExitRequested>,
    controller: SystemController,
    project_snapshot: Option<NormalizedProjectSnapshot>,
    selected_locale: String,
    available_fonts: BTreeSet<String>,
    save_extensions: Vec<era_runtime_save::OpaqueSaveExtension>,
    system_menu: SystemMenuState,
    load_slot_paths: Vec<String>,
    inbound_transfer: Option<InboundStateTransfer>,
    outbound_transfer: Option<OutboundStateTransfer>,
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
            outbound_sequence: 0,
            next_message_id: 1,
            next_request_id: 1,
            next_wait_id: 1,
            next_interaction_id: 1,
            next_transfer_id: 1,
            logical_time_ns: 0,
            frontend_time_origin: None,
            random_seed: None,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            outbound_journal: BTreeMap::new(),
            accepted_message_ids: BTreeMap::new(),
            artifact: None,
            incremental: IncrementalState::default(),
            vm: None,
            presentation: PresentationModel::default(),
            operations: PendingOperations::default(),
            key_toggle_state: [0; 256],
            message_skip: false,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            exit_requested: None,
            controller: SystemController::default(),
            project_snapshot: None,
            selected_locale: "ja".into(),
            available_fonts: BTreeSet::new(),
            save_extensions: Vec::new(),
            system_menu: SystemMenuState::Title,
            load_slot_paths: Vec::new(),
            inbound_transfer: None,
            outbound_transfer: None,
        }
    }

    /// Decode and queue one frontend envelope without executing runtime work.
    ///
    /// # Errors
    ///
    /// Rejects malformed, oversized, out-of-sequence, or wrong-session envelopes.
    pub fn submit_envelope(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        let envelope = decode_envelope(bytes, self.options.wire_limits)?;
        if self.state == SessionState::Active
            && (envelope.session != Some(self.options.session_id)
                || envelope.session_epoch != Some(self.epoch))
        {
            return Err(RuntimeError::SessionMismatch);
        }
        let envelope_hash = blake3::hash(bytes);
        if envelope.sequence < self.expected_inbound_sequence {
            if self.accepted_message_ids.get(&envelope.message_id)
                == Some(&(envelope.sequence, envelope_hash))
            {
                return Ok(());
            }
            return Err(RuntimeError::InvalidSequence {
                expected: self.expected_inbound_sequence,
                actual: envelope.sequence,
            });
        }
        if envelope.sequence != self.expected_inbound_sequence {
            return Err(RuntimeError::InvalidSequence {
                expected: self.expected_inbound_sequence,
                actual: envelope.sequence,
            });
        }
        if self.inbound.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("inbound journal is full"));
        }
        let message_id = envelope.message_id;
        let message = RuntimeMessage::from_envelope(&envelope)?;
        self.expected_inbound_sequence = self.expected_inbound_sequence.saturating_add(1);
        self.accepted_message_ids
            .insert(message_id, (envelope.sequence, envelope_hash));
        while self.accepted_message_ids.len() > self.options.limits.maximum_journal_entries as usize
        {
            self.accepted_message_ids.pop_first();
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
                self.handle_message(message_id, message)?;
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
        match message {
            RuntimeMessage::ProjectManifest(manifest) => self.load_project(message_id, &manifest),
            RuntimeMessage::Start(start) => self.start(message_id, &start),
            RuntimeMessage::Input(input) => self.complete_input(message_id, input),
            RuntimeMessage::AdvanceTime(time) => self.advance_time(message_id, time),
            RuntimeMessage::DeviceStateChanged(state) => {
                self.observe_frontend_time(state.monotonic_time_ns);
                Ok(())
            }
            RuntimeMessage::ClientStateChanged(_) | RuntimeMessage::EffectAcknowledgement(_) => {
                Ok(())
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
            RuntimeMessage::Resynchronize(_) => self.emit(
                RuntimeMessage::RuntimeResynchronized(RuntimeResynchronized {
                    epoch: self.epoch.0,
                    phase: self.phase,
                    runtime_revision: self.revision,
                    presentation: self.presentation.snapshot(),
                    exit_requested: self.exit_requested,
                    selected_locale: self.selected_locale.clone(),
                }),
                Some(message_id),
            ),
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
            | RuntimeMessage::RuntimeResynchronized(_) => self.reject(
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
                    message: "runtime protocol 9.0 is required".into(),
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
        let build = build_project(manifest, Some(&self.incremental));
        let success = build.report.success;
        self.incremental = build.incremental;
        self.artifact = build.artifact;
        self.project_snapshot = build.snapshot;
        if let Some(snapshot) = &self.project_snapshot {
            self.presentation.configure_layout(
                snapshot.viewport_width,
                snapshot.print_c_per_line,
                snapshot.print_c_length,
            );
        }
        self.emit(
            RuntimeMessage::ProjectLoadReport(build.report),
            Some(message_id),
        )?;
        self.set_phase(if success {
            RuntimePhase::Ready
        } else {
            RuntimePhase::Faulted
        })
    }

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
        self.emit(
            RuntimeMessage::ProjectLoadReport(build.report),
            Some(message_id),
        )?;
        self.set_phase(previous_phase)?;
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
        self.advance_epoch();
        self.set_phase(RuntimePhase::Starting)?;
        if self.controller.prepare_load_sequence(vm.vm().artifact()) {
            self.spawn_next_event(&mut vm)?;
        }
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
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
        self.controller = payload.controller;
        self.logical_time_ns = payload.logical_time_ns;
        self.frontend_time_origin = None;
        self.random_seed = payload.random_seed;
        self.message_skip = payload.message_skip;
        self.command_intents = remap_intents(payload.command_intents);
        self.reusable_system_intents = remap_intents(payload.reusable_system_intents);
        self.save_extensions = payload.save_extensions;
        self.system_menu = system_menu;
        self.load_slot_paths = payload.load_slot_paths;
        self.set_phase(RuntimePhase::WaitingInput)?;
        self.emit_presentation()
    }

    fn start_new_game(&mut self, seed: u64) -> Result<(), RuntimeError> {
        self.random_seed = Some(seed);
        self.frontend_time_origin = None;
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
        if self
            .controller
            .prepare_function(vm.vm().artifact(), "SYSTEM_TITLE")
        {
            self.spawn_next_event(&mut vm)?;
            self.vm = Some(vm);
            self.set_phase(RuntimePhase::Running)
        } else {
            self.vm = Some(vm);
            self.open_title_menu()
        }
    }

    fn open_title_menu(&mut self) -> Result<(), RuntimeError> {
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths.clear();
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
                host_request: None,
                wait,
                result_name: None,
                choices: BTreeMap::from([
                    (start_token, VmValue::Integer(0)),
                    (load_token, VmValue::Integer(1)),
                ]),
                timeout_duration_ns: None,
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
            VmPortEvent::FiberFaulted(_, fault) => {
                self.fault(FaultCode::VmFault, &fault.message, None)
            }
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
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_host_call(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
    ) -> Result<(), RuntimeError> {
        let name = request.import.import.name.to_ascii_uppercase();
        if name == "AWAIT" {
            let milliseconds = match request.arguments.first() {
                None | Some(VmValue::Integer(0)) => 0,
                Some(VmValue::Integer(value @ 1..=10_000)) => *value,
                _ => {
                    return self.fault(
                        FaultCode::VmFault,
                        "AWAIT duration must be between 0 and 10000 milliseconds",
                        None,
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
                    None,
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
                return self.fault(FaultCode::VmFault, "BEGIN expects a system keyword", None);
            };
            let Some(flow) = SystemFlow::parse(keyword) else {
                return self.fault(
                    FaultCode::VmFault,
                    &format!("unknown BEGIN system keyword: {keyword}"),
                    None,
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
                None,
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
                    None,
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
                    None,
                );
            };
            let value = match logical_line_string(pattern, 75) {
                Ok(value) => value,
                Err(message) => return self.fault(FaultCode::VmFault, message, None),
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
                None,
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
        if name == "PRINT_IMG" {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            self.presentation.append_image(resource, None);
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
        if matches!(name.as_str(), "PLAYBGM" | "PLAYSOUND") {
            let resource = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            self.presentation
                .set_audio(resource, name == "PLAYBGM", true);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(name.as_str(), "STOPBGM" | "STOPSOUND") {
            self.presentation
                .set_audio(String::new(), name == "STOPBGM", false);
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "PRINTBUTTON" | "PRINTBUTTONC" | "PRINTBUTTONLC"
        ) {
            let text = request
                .arguments
                .first()
                .map_or_else(String::new, display_value);
            let value = request
                .arguments
                .get(1)
                .cloned()
                .ok_or_else(|| RuntimeError::Internal("PRINTBUTTON value is missing".into()))?;
            let token = self.allocate_interaction();
            self.presentation.append_button(text, token);
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
            if is_column_print(&name) {
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
                FaultCode::VmFault,
                &format!("unsupported host import: {}", request.import.import.name),
                None,
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

    fn issue_storage(
        &mut self,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        let request_id = self.allocate_request()?;
        self.operations.insert_storage(request_id, pending);
        self.set_phase(RuntimePhase::Paused)?;
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
            (PendingStorage::BuiltinAutosave, StorageResult::Written { .. }) => {
                self.finish_builtin_autosave(true)
            }
            (PendingStorage::BuiltinAutosave, StorageResult::Error { .. }) => {
                self.finish_builtin_autosave(false)
            }
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
            (PendingStorage::ListLoadSlots, StorageResult::Listed { mut entries }) => {
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
                self.system_menu = SystemMenuState::LoadSlots;
                // SaveDataNos is the total ordinary slot count. The reference
                // system menu always paginates in groups of twenty.
                let maximum = 20;
                self.load_slot_paths = entries
                    .into_iter()
                    .take(maximum)
                    .map(|entry| entry.relative_path)
                    .collect();
                self.presentation.append_system_text(
                    localized_system_text(&self.selected_locale, SystemTextKey::LoadQuestion),
                    SystemTextKey::LoadQuestion,
                    Vec::new(),
                    false,
                );
                let mut choices = BTreeMap::new();
                for index in 0..self.load_slot_paths.len() {
                    let path = self.load_slot_paths[index].clone();
                    let token = self.allocate_interaction();
                    let arguments = vec![SystemTextArgument::String(path.clone())];
                    self.presentation.append_system_button(
                        format!(
                            "{}: {path}",
                            localized_system_text(&self.selected_locale, SystemTextKey::SaveSlot)
                        ),
                        SystemTextKey::SaveSlot,
                        arguments,
                        token,
                    );
                    choices.insert(
                        token,
                        VmValue::Integer(
                            i64::try_from(index).unwrap_or(i64::MAX).saturating_add(2),
                        ),
                    );
                }
                let back = self.allocate_interaction();
                self.presentation.append_system_button(
                    localized_system_text(&self.selected_locale, SystemTextKey::Back),
                    SystemTextKey::Back,
                    Vec::new(),
                    back,
                );
                choices.insert(back, VmValue::Integer(-1));
                let submission = self.allocate_interaction();
                let wait = self.system_wait(submission);
                self.open_wait(
                    PendingInput {
                        host_request: None,
                        wait,
                        result_name: None,
                        choices,
                        timeout_duration_ns: None,
                    },
                    true,
                )
            }
            (PendingStorage::ReadLoadSlot, StorageResult::Read { data, .. }) => {
                self.set_phase(RuntimePhase::Ready)?;
                self.start_traditional_save(message_id, data.as_slice())
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
                        | PendingStorage::BuiltinAutosave
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
                self.open_title_menu()
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
        self.set_phase(RuntimePhase::Running)
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
        self.controller.prepare_load_sequence(vm.vm().artifact());
        self.spawn_next_event(&mut vm)?;
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
                };
                let host_request = match completion {
                    ExternalCompletion::GetKey { request: id, .. }
                    | ExternalCompletion::LocalDateTime { request: id, .. } => id,
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
        }
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
            self.activate_wait(next, pause_next_wait)
        } else {
            self.set_phase(RuntimePhase::Running)
        }
    }

    fn finish_system_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        if self.controller.step != SystemStep::None {
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
            (SystemMenuState::LoadSlots, VmValue::Integer(-1)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.open_title_menu()
            }
            (SystemMenuState::LoadSlots, VmValue::Integer(selection)) if *selection >= 2 => {
                let index = usize::try_from(*selection - 2).unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                };
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot,
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
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
                        let description = read_runtime_string(vm, "SAVEDATA_TEXT")?;
                        let bytes = encode_scoped_save(
                            &vm.export_era_state(),
                            vm.vm().artifact(),
                            era_runtime_save::SaveFileKind::Normal,
                            description,
                            merge_structured_extensions(
                                &self.save_extensions,
                                vm.structured_extensions(StructuredScope::Ordinary)
                                    .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                            )
                            .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                            self.traditional_save_format(),
                        )
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                        return self.issue_storage(
                            PendingStorage::BuiltinAutosave,
                            StorageNamespace::Save,
                            StorageOperation::Write {
                                data: ProtocolBytes::new(bytes),
                                atomic_replace: true,
                                precondition: StoragePrecondition::Any,
                            },
                            save_slot_path(99),
                        );
                    }
                } else {
                    self.controller.step = SystemStep::ShopShow;
                    self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
                }
            }
            SystemStep::ShopAutosave | SystemStep::ShopAction => {
                self.controller.step = SystemStep::ShopShow;
                self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
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
                .append_button(format!("{name}[{display:>3}]"), token);
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
        if self.operations.has_transient_external() {
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
                        epoch: self.epoch.0,
                        vm_snapshot,
                        presentation: self.presentation.clone(),
                        operations: self.operations.clone(),
                        controller: self.controller.clone(),
                        logical_time_ns: self.logical_time_ns,
                        random_seed: self.random_seed,
                        message_skip: self.message_skip,
                        command_intents: self.command_intents.clone(),
                        reusable_system_intents: self.reusable_system_intents.clone(),
                        save_extensions: self.save_extensions.clone(),
                        system_menu: match self.system_menu {
                            SystemMenuState::Title => 0,
                            SystemMenuState::LoadSlots => 1,
                        },
                        load_slot_paths: self.load_slot_paths.clone(),
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
        self.set_phase(RuntimePhase::Stopping)?;
        let cancelled = self.operations.total_count();
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

    fn emit_presentation(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::PresentationSnapshot(self.presentation.snapshot()),
            None,
        )
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
        source: Option<era_runtime_protocol::SourceLocation>,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Fault(RuntimeFault {
                code,
                message: message.into(),
                source,
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

fn is_print(name: &str) -> bool {
    name.starts_with("PRINT") || name == "REUSELASTLINE"
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
    }
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

#[cfg(test)]
mod tests {
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
                                "@SYSTEM_TITLE\nINPUT\nZZZSAVE = RESULT\nINPUT\nRETURN\n@SYSTEM_LOADEND\nPRINTFORML restored={ZZZSAVE}\nRETURN\n"
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
        assert!(output.iter().any(|message| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => snapshot.lines.iter().any(|line| {
                line.runs.iter().any(|run| {
                    matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text { text, .. }
                            if text.contains("restored=37")
                    )
                })
            }),
            _ => false,
        }));

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
    fn storage_slot_listing_is_sorted_and_runtime_tokenized() {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.state = SessionState::Active;
        session.phase = RuntimePhase::Paused;
        session.epoch = SessionEpoch(1);
        session.selected_locale = "en".into();
        session
            .operations
            .insert_storage(7, PendingStorage::ListLoadSlots);
        session
            .complete_storage(
                10,
                StorageResponse {
                    request_id: 7,
                    result: StorageResult::Listed {
                        entries: vec![
                            era_runtime_protocol::StorageEntry {
                                relative_path: "save02.sav".into(),
                                byte_length: 20,
                                revision: None,
                            },
                            era_runtime_protocol::StorageEntry {
                                relative_path: "save01.sav".into(),
                                byte_length: 10,
                                revision: None,
                            },
                        ],
                    },
                },
            )
            .unwrap();
        assert_eq!(
            session.load_slot_paths,
            ["save01.sav".to_owned(), "save02.sav".to_owned()]
        );
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
        assert_eq!(session.phase(), RuntimePhase::Paused);

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
                        payload: FilePayload::Utf8("name,path".into()),
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
            *blake3::hash(b"name,path").as_bytes()
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
    fn font_profile_is_session_fixed_case_insensitive_and_deterministic() {
        let mut requested = capabilities();
        requested.available_fonts = vec!["Zeta".into(), "alpha".into(), "ALPHA".into()];
        let selected = selected_capabilities(&requested);
        assert_eq!(selected.available_fonts, vec!["alpha", "Zeta"]);
    }
}
