use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use era_protocol::{
    ProtocolBytes, ProtocolError, ProtocolVersion, SessionEpoch, SessionId, VersionRange,
    WireLimits, decode_canonical, decode_envelope, encode_canonical, encode_envelope,
    negotiate_version,
};
use era_runtime_protocol::{
    AdvanceTime, CellAlignment, ClientCapabilities, ClientHello, CommandErrorCode, CommandRejected,
    FaultCode, FrontendInput, GET_KEY_STATE_OPERATION, GET_KEY_STATE_OPERATION_VERSION,
    GetKeyStateRequest, GetKeyStateResponse, InputIntent, InputWait, InteractionToken,
    LOCAL_DATE_TIME_OPERATION, LOCAL_DATE_TIME_OPERATION_VERSION, LocalDateTimeRequest,
    LocalDateTimeResponse, ProjectManifest, RANDOM_SEED_OPERATION, RANDOM_SEED_OPERATION_VERSION,
    RUNTIME_PROTOCOL_VERSION, RandomSeedRequest, RandomSeedResponse, RuntimeFault, RuntimeFeature,
    RuntimeLimits, RuntimeMessage, RuntimePhase, RuntimeResynchronized, RuntimeStateChanged,
    ServerHello, ServiceKind, ServiceRequest, ServiceResponse, ServiceResult, ShutdownReady,
    StartMode, StartRequest, StateExportReady, StateExportRequest, StateExportResult,
    VersionRejected, WaitChange, WaitKind, WaitStability,
};
use erabasic_bytecode::SymbolKey;
use erabasic_compiler::IncrementalState;
use erabasic_validator::ValidatedArtifact;
use erabasic_vm::{
    HostReady, HostWaitStability, HostWrite, PlaceDescriptor, RunBudget, RuntimeVm, VmConfig,
    VmDriveMode, VmHostCompletion, VmHostRequest, VmPortEvent, VmPortStop, VmRuntimePort,
    VmRuntimeStatePort, VmRuntimeStateTransaction, VmValue,
};

use crate::host::{ClockOperation, ExternalCompletion, PendingInput, input_wait};
use crate::presentation::{PresentationModel, display_value, logical_line_string};
use crate::project::build_project;

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

#[derive(Clone, Debug)]
enum PendingService {
    StartEntropy,
    Host(ExternalCompletion),
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
    logical_time_ns: u64,
    random_seed: Option<u64>,
    inbound: VecDeque<(u64, RuntimeMessage)>,
    outbound: VecDeque<Vec<u8>>,
    outbound_journal: BTreeMap<u64, Vec<u8>>,
    accepted_message_ids: BTreeMap<u64, (u64, blake3::Hash)>,
    artifact: Option<ValidatedArtifact>,
    incremental: IncrementalState,
    vm: Option<RuntimeVm>,
    presentation: PresentationModel,
    pending_input: Option<PendingInput>,
    queued_inputs: VecDeque<PendingInput>,
    pending_services: BTreeMap<u64, PendingService>,
    pending_delays: BTreeMap<erabasic_vm::HostRequestId, u64>,
    key_toggle_state: [u8; 256],
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
            logical_time_ns: 0,
            random_seed: None,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            outbound_journal: BTreeMap::new(),
            accepted_message_ids: BTreeMap::new(),
            artifact: None,
            incremental: IncrementalState::default(),
            vm: None,
            presentation: PresentationModel::default(),
            pending_input: None,
            queued_inputs: VecDeque::new(),
            pending_services: BTreeMap::new(),
            pending_delays: BTreeMap::new(),
            key_toggle_state: [0; 256],
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
                if self.pending_input.is_some()
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
                self.logical_time_ns = self.logical_time_ns.max(state.monotonic_time_ns);
                Ok(())
            }
            RuntimeMessage::ClientStateChanged(_) | RuntimeMessage::EffectAcknowledgement(_) => {
                Ok(())
            }
            RuntimeMessage::ServiceResponse(response) => {
                self.complete_service(message_id, response)
            }
            RuntimeMessage::StateExportRequest(request) => self.export_state(message_id, request),
            RuntimeMessage::ReloadProject(_) => self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "hot reload is not enabled by this runtime stage",
            ),
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
                }),
                Some(message_id),
            ),
            RuntimeMessage::StorageResponse(_) => self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "no storage request is pending",
            ),
            RuntimeMessage::ClientHello(_)
            | RuntimeMessage::ServerHello(_)
            | RuntimeMessage::VersionRejected(_)
            | RuntimeMessage::ProjectLoadReport(_)
            | RuntimeMessage::StateChanged(_)
            | RuntimeMessage::WaitChanged(_)
            | RuntimeMessage::PresentationSnapshot(_)
            | RuntimeMessage::PresentationDelta(_)
            | RuntimeMessage::EffectBatch(_)
            | RuntimeMessage::StorageRequest(_)
            | RuntimeMessage::ServiceRequest(_)
            | RuntimeMessage::StateExportReady(_)
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
                    message: "runtime protocol 4.0 is required".into(),
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
            RuntimeFeature::TimedInput,
            RuntimeFeature::ExternalServices,
            RuntimeFeature::StateResynchronization,
        ];
        let features = implemented
            .into_iter()
            .filter(|feature| hello.features.contains(feature))
            .collect();
        let selected_capabilities = selected_capabilities(&hello.capabilities);
        self.presentation.set_projection(
            selected_capabilities.column_cells,
            selected_capabilities.separators,
        );
        self.emit(
            RuntimeMessage::ServerHello(ServerHello {
                selected_version: selected,
                session: self.options.session_id,
                features,
                limits,
                epoch: self.epoch.0,
                selected_capabilities,
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
                self.pending_services
                    .insert(request_id, PendingService::StartEntropy);
                self.emit(
                    RuntimeMessage::ServiceRequest(ServiceRequest {
                        request_id,
                        kind: ServiceKind::Entropy,
                        operation: RANDOM_SEED_OPERATION.into(),
                        operation_version: RANDOM_SEED_OPERATION_VERSION,
                        payload: ProtocolBytes::new(encode_canonical(&RandomSeedRequest {})?),
                    }),
                    Some(message_id),
                )
            }
            StartMode::TraditionalSave { .. } | StartMode::VmSnapshot { .. } => self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "save restoration is reserved by the protocol but not implemented yet",
            ),
        }
    }

    fn start_new_game(&mut self, seed: u64) -> Result<(), RuntimeError> {
        self.random_seed = Some(seed);
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
        let entry = function_key(&artifact, "SYSTEM_TITLE");
        self.presentation.set_title(title);
        let mut vm = RuntimeVm::new_with_seed(artifact, self.options.vm_config, seed);
        let prepared = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        if let Some(entry) = entry {
            vm.spawn_entry(entry, Vec::new())
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.vm = Some(vm);
            self.set_phase(RuntimePhase::Running)
        } else {
            self.vm = Some(vm);
            self.presentation
                .append_text("[0] Start a new game".into(), false);
            let start_token = self.allocate_interaction();
            let load_token = self.allocate_interaction();
            let submission_token = self.allocate_interaction();
            self.presentation
                .append_button("Start a new game".into(), start_token);
            self.presentation
                .append_button("Load game".into(), load_token);
            let wait = InputWait {
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
            };
            let choices = BTreeMap::from([
                (start_token, VmValue::Integer(0)),
                (load_token, VmValue::Integer(1)),
            ]);
            self.open_wait(
                PendingInput {
                    host_request: None,
                    wait,
                    result_name: None,
                    choices,
                },
                true,
            )
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
            VmPortEvent::FiberYielded(_) | VmPortEvent::FiberCompleted(_, _) => Ok(()),
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
            self.pending_delays.insert(
                request.id,
                self.logical_time_ns
                    .saturating_add(milliseconds.cast_unsigned().saturating_mul(1_000_000)),
            );
            return Ok(());
        }
        if let Some(pending) = input_wait(
            request,
            self.allocate_wait(),
            self.allocate_interaction(),
            self.logical_time_ns,
        ) {
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
                };
                let pending = PendingInput {
                    host_request: Some(request.id),
                    wait,
                    result_name: None,
                    choices: BTreeMap::new(),
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
        self.pending_services
            .insert(request_id, PendingService::Host(completion));
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind,
                operation: operation.into(),
                operation_version,
                payload: ProtocolBytes::new(encode_canonical(payload)?),
            }),
            None,
        )
    }

    fn complete_service(
        &mut self,
        message_id: u64,
        response: ServiceResponse,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.pending_services.remove(&response.request_id) else {
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
        let Some(pending) = self.pending_input.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "no input is pending",
            );
        };
        if pending.wait.wait_id != input.wait_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "input wait identity is stale",
            );
        }
        if pending
            .wait
            .deadline_ns
            .is_some_and(|deadline| input.monotonic_time_ns > deadline)
        {
            return self.advance_time(
                message_id,
                AdvanceTime {
                    monotonic_time_ns: input.monotonic_time_ns,
                },
            );
        }
        let Some(submission) = input_value(pending, input.token, input.intent) else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "input value does not match the active wait",
            );
        };
        self.logical_time_ns = self.logical_time_ns.max(input.monotonic_time_ns);
        self.finish_input(submission, false)
    }

    fn advance_time(&mut self, _message_id: u64, time: AdvanceTime) -> Result<(), RuntimeError> {
        self.logical_time_ns = self.logical_time_ns.max(time.monotonic_time_ns);
        let ready_delays: Vec<_> = self
            .pending_delays
            .iter()
            .filter_map(|(request, deadline)| {
                (*deadline <= self.logical_time_ns).then_some(*request)
            })
            .collect();
        for request in ready_delays {
            self.pending_delays.remove(&request);
            let vm = self
                .vm
                .as_mut()
                .ok_or_else(|| RuntimeError::Internal("pending AWAIT has no VM".into()))?;
            commit_completion(vm, request, VmHostCompletion::Ready(HostReady::empty()))?;
            self.set_phase(RuntimePhase::Running)?;
        }
        let timed_out = self
            .pending_input
            .as_ref()
            .and_then(|pending| pending.wait.deadline_ns)
            .is_some_and(|deadline| self.logical_time_ns >= deadline);
        if timed_out {
            let pending = self.pending_input.as_ref().expect("checked above");
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
            .pending_input
            .take()
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
                if let Some(target) = pending.result_name.and_then(|name| global_place(vm, name)) {
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
        if let Some(next) = self.queued_inputs.pop_front() {
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
        match value {
            VmValue::Integer(0) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                let entry = vm
                    .vm()
                    .artifact()
                    .functions
                    .iter()
                    .find(|function| function.name.eq_ignore_ascii_case("EVENTFIRST"))
                    .map(|function| function.key)
                    .ok_or_else(|| RuntimeError::Internal("EVENTFIRST is not defined".into()))?;
                vm.spawn_entry(entry, Vec::new())
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                self.set_phase(RuntimePhase::Running)
            }
            VmValue::Integer(1) => {
                self.pending_input = Some(pending);
                self.reject(
                    0,
                    CommandErrorCode::FeatureUnavailable,
                    "traditional save selection is not implemented yet",
                )
            }
            _ => {
                self.pending_input = Some(pending);
                self.reject(
                    0,
                    CommandErrorCode::InvalidValue,
                    "unknown system menu item",
                )
            }
        }
    }

    fn open_wait(
        &mut self,
        pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if self.pending_input.is_some() {
            self.queued_inputs.push_back(pending);
            return Ok(());
        }
        self.activate_wait(pending, pause_runtime)
    }

    fn activate_wait(
        &mut self,
        pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        self.presentation.set_wait(Some(pending.wait.clone()));
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Opened(pending.wait.clone())),
            None,
        )?;
        self.pending_input = Some(pending);
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

    fn export_state(
        &mut self,
        message_id: u64,
        request: StateExportRequest,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: request.kind,
                result: StateExportResult::Ineligible {
                    reasons: vec![
                        "the traditional save codec and runtime snapshot container are not implemented"
                            .into(),
                    ],
                },
            }),
            Some(message_id),
        )
    }

    fn shutdown(&mut self, message_id: u64) -> Result<(), RuntimeError> {
        self.set_phase(RuntimePhase::Stopping)?;
        let cancelled = self.pending_services.len()
            + self.pending_delays.len()
            + self.queued_inputs.len()
            + usize::from(self.pending_input.is_some());
        self.pending_services.clear();
        self.pending_delays.clear();
        self.pending_input = None;
        self.queued_inputs.clear();
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
        if self.pending_services.len() >= self.options.limits.maximum_pending_requests as usize {
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
        self.next_interaction_id = 1;
        self.accepted_message_ids.clear();
    }
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

fn function_key(artifact: &ValidatedArtifact, name: &str) -> Option<SymbolKey> {
    artifact
        .artifact()
        .functions
        .iter()
        .find(|function| function.name.eq_ignore_ascii_case(name))
        .map(|function| function.key)
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
    // Stage one exposes the normalized text projection only. Capabilities are
    // intersected here instead of echoing unsupported renderer promises.
    ClientCapabilities {
        input_modalities: client.input_modalities.clone(),
        rich_text: false,
        html: false,
        graphics: false,
        audio: false,
        video: false,
        font_metrics: false,
        column_cells: client.column_cells,
        separators: client.separators,
    }
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
    }
}

#[cfg(test)]
mod tests {
    use era_protocol::{Channel, Envelope, ProtocolBytes, decode_envelope, encode_envelope};
    use era_runtime_protocol::{FileCategory, FilePayload, ProjectManifest, SubmittedFile};

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
    fn typed_input_atomically_updates_result_and_resumes_the_vm() {
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
                        "@SYSTEM_TITLE\nTINPUT 1000, 7, 1, \"timeout\", 0, 0\nPRINTFORML got={RESULT}\nRETURN\n"
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
                    era_runtime_protocol::DisplayRun::Text { text, .. } if text.contains("got=42")
                ))
            }),
            _ => false,
        }));
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
    fn deferred_configuration_and_resources_are_reported_instead_of_ignored() {
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
        assert!(codes.contains(&"runtime.configuration_deferred"));
        assert!(codes.contains(&"runtime.resource_manifest_deferred"));
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
            },
            result_name: Some("RESULT"),
            choices: BTreeMap::from([(selection, VmValue::Integer(42))]),
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
}
