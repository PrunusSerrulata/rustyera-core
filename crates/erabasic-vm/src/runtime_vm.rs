use erabasic_bytecode::{Digest, HostImport, HostSnapshotCapability, SymbolKey};
use erabasic_validator::ValidatedArtifact;

use crate::structured::{StructuredExtension, StructuredScope};
use crate::{
    EraState, EraStateReport, FiberId, FiberState, FiberStatus, GenerationId, HostCallRequest,
    HostCallResult, HostReady, HostRequestId, HostWaitStability, HotReloadReport,
    ImmediateHostCall, ImmediateHostCallResult, NativeServiceRegistry, PlaceDescriptor,
    PreparedRuntimeState, RunBudget, SnapshotEligibility, Vm, VmConfig, VmDriveMode, VmError,
    VmHost, VmHostCompletion, VmHostRequest, VmPortDriveReport, VmPortEvent, VmPortStop,
    VmRestorePort, VmRuntimePort, VmRuntimeRead, VmRuntimeStatePort, VmRuntimeStateTransaction,
    VmSnapshot, VmValue, VmWaitRebind,
};
use std::collections::BTreeSet;

use crate::debug::DebugState;

/// Runtime-facing VM owner. It keeps native services beside the interpreter so the
/// caller-pumped runtime port never needs a callback parameter.
pub struct RuntimeVm {
    vm: Vm,
    natives: NativeServiceRegistry,
    pending_natives: Option<NativeServiceRegistry>,
    line_columns: u32,
}

/// Stable logical width used until a frontend reports its projection dimensions.
pub const DEFAULT_LINE_COLUMNS: u32 = 75;

/// Opaque candidate state prepared against one exact artifact generation.
/// It intentionally excludes fibers, frames and scheduler counters.
pub struct PreparedCandidateState {
    artifact_id: Digest,
    memory: crate::Memory,
    natives: NativeServiceRegistry,
}

impl RuntimeVm {
    /// Read a place supplied to a Host extension without exposing VM storage layouts.
    ///
    /// # Errors
    ///
    /// The place must still belong to the requesting fiber and current generation.
    pub fn read_host_place(
        &self,
        fiber: FiberId,
        place: &PlaceDescriptor,
    ) -> Result<VmValue, VmError> {
        let fiber = self
            .vm
            .fibers
            .get(&fiber)
            .ok_or_else(|| VmError::InvalidState("Host place fiber is missing".into()))?;
        self.vm.read_place(fiber, place)
    }
    /// Fork authoritative memory and Native state while discarding every live
    /// fiber. Candidate SAVEINFO execution uses this isolated timeline so a
    /// failure cannot leak stack, scheduler, random or structured state.
    ///
    /// # Errors
    ///
    /// Returns an error when a registered Native service cannot be snapshotted.
    pub fn fork_isolated(&self) -> Result<Self, VmError> {
        let mut vm = self.vm.clone();
        vm.fibers.clear();
        vm.runnable.clear();
        vm.primary_fiber = None;
        vm.next_fiber = 1;
        vm.pending_reload = None;
        vm.debug = DebugState::default();
        vm.path_memo_cache.clear();
        vm.path_memo_retained_bytes = 0;
        vm.active_path_memo_fiber.set(None);
        vm.active_path_memo.borrow_mut().take();
        let natives = self
            .natives
            .fork_for_artifact(vm.artifact())
            .map_err(VmError::Snapshot)?;
        Ok(Self {
            vm,
            natives,
            pending_natives: None,
            line_columns: self.line_columns,
        })
    }

    #[must_use]
    pub fn into_candidate_state(self) -> PreparedCandidateState {
        PreparedCandidateState {
            artifact_id: self.vm.artifact_id(),
            memory: self.vm.memory,
            natives: self.natives,
        }
    }

    /// Atomically install candidate memory and Native services without replacing
    /// the caller's fibers or call stacks.
    ///
    /// # Errors
    ///
    /// Rejects a candidate prepared for another artifact generation.
    pub fn commit_candidate_state(
        &mut self,
        candidate: PreparedCandidateState,
    ) -> Result<(), VmError> {
        if candidate.artifact_id != self.vm.artifact_id() {
            return Err(VmError::InvalidState(
                "candidate state belongs to another artifact".into(),
            ));
        }
        self.vm.memory = candidate.memory;
        self.natives = candidate.natives;
        self.refresh_draw_line_string();
        Ok(())
    }

    #[must_use]
    pub fn new(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        let natives = NativeServiceRegistry::for_artifact(artifact.artifact());
        let mut runtime = Self {
            vm: Vm::new(artifact, config),
            natives,
            pending_natives: None,
            line_columns: DEFAULT_LINE_COLUMNS,
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    #[must_use]
    pub fn new_with_seed(artifact: ValidatedArtifact, config: VmConfig, seed: u64) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        let mut runtime = Self {
            vm: Vm::new(artifact, config),
            natives,
            pending_natives: None,
            line_columns: DEFAULT_LINE_COLUMNS,
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    /// Construct the pre-title state used by the runtime system flow.
    ///
    /// Variable defaults are available to `SYSTEM_TITLE`, while `ResetData` and
    /// initial character insertion remain deferred until the built-in new-game
    /// selection is accepted.
    #[must_use]
    pub fn new_for_title_with_seed(
        artifact: ValidatedArtifact,
        config: VmConfig,
        seed: u64,
    ) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        let mut runtime = Self {
            vm: Vm::new_for_title(artifact, config),
            natives,
            pending_natives: None,
            line_columns: DEFAULT_LINE_COLUMNS,
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    #[must_use]
    pub fn new_for_title_with_seed_and_progress(
        artifact: ValidatedArtifact,
        config: VmConfig,
        seed: u64,
        progress: &mut dyn FnMut(crate::VmPreparationProgress),
    ) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        let mut runtime = Self {
            vm: Vm::new_for_title_with_progress(artifact, config, progress),
            natives,
            pending_natives: None,
            line_columns: DEFAULT_LINE_COLUMNS,
        };
        runtime.refresh_draw_line_string();
        runtime
    }

    /// Synchronize calculated line-width state with the current frontend projection.
    pub fn set_line_columns(&mut self, columns: u32) {
        self.line_columns = columns.max(1);
        self.refresh_draw_line_string();
    }

    /// Synchronize runtime formatting and calculated strings with project width policy.
    pub fn set_character_width_mode(&mut self, mode: crate::CharacterWidthMode) {
        self.natives.set_character_width_mode(mode);
        if let Some(pending) = &mut self.pending_natives {
            pending.set_character_width_mode(mode);
        }
        self.refresh_draw_line_string();
    }

    #[must_use]
    pub fn character_width_mode(&self) -> crate::CharacterWidthMode {
        self.natives.character_width_mode()
    }

    fn refresh_draw_line_string(&mut self) {
        let pattern = self
            .vm
            .artifact()
            .project_data
            .static_data
            .replace
            .draw_line_string
            .clone();
        let value = crate::logical_line_string_with_mode(
            &pattern,
            usize::try_from(self.line_columns).unwrap_or(usize::MAX),
            self.character_width_mode(),
        )
        .unwrap_or(pattern);
        self.vm.set_runtime_calculated_string("DRAWLINESTR", &value);
    }

    fn current_host_import(&self, key: SymbolKey) -> Option<&HostImport> {
        let generation = self.vm.generations.get(&self.vm.current_generation)?;
        let index = generation.host_import_index(key)?;
        generation.artifact.host_imports.get(index)
    }

    #[must_use]
    pub const fn vm(&self) -> &Vm {
        &self.vm
    }

    pub const fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    #[must_use]
    pub fn fiber_frame_count(&self, fiber: FiberId) -> Option<usize> {
        self.vm.fibers.get(&fiber).map(|fiber| fiber.frames.len())
    }

    /// Return the active dimensions for a named global, local, or bound
    /// reference variable in the requesting fiber.
    #[must_use]
    pub fn variable_dimensions(&self, fiber: FiberId, name: &str) -> Option<Vec<u64>> {
        self.vm.variable_dimensions(fiber, name)
    }

    /// Return the active dimensions for a place supplied by a Host call.
    ///
    /// # Errors
    ///
    /// The place must still belong to the requesting fiber and resolve to an
    /// active variable.
    pub fn host_place_dimensions(
        &self,
        fiber: FiberId,
        place: &PlaceDescriptor,
    ) -> Result<Vec<u64>, VmError> {
        self.vm.host_place_dimensions(fiber, place)
    }

    /// Whether at least one fiber can make progress without a host completion.
    #[must_use]
    pub fn has_runnable_fibers(&self) -> bool {
        self.vm
            .fibers
            .values()
            .any(|fiber| matches!(fiber.state, FiberState::Runnable))
    }

    /// Export the exact SFMT stream position used by RAND natives.
    ///
    /// # Errors
    ///
    /// Returns an error if the native random state is unavailable or poisoned.
    pub fn export_random_state(&self) -> Result<Vec<i64>, VmError> {
        self.natives.random_values().map_err(VmError::InvalidState)
    }

    /// Restore a state previously returned by `export_random_state`.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded SFMT state is invalid or unavailable.
    pub fn restore_random_state(&mut self, values: &[i64]) -> Result<(), VmError> {
        self.natives
            .set_random_values(values)
            .map_err(VmError::InvalidState)
    }

    /// Export only VAREXT values declared for the requested save scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the shared structured state cannot be serialized.
    pub fn structured_extensions(
        &self,
        scope: StructuredScope,
    ) -> Result<Vec<StructuredExtension>, VmError> {
        self.natives
            .structured_extensions(scope)
            .map_err(VmError::InvalidState)
    }

    /// Prepare ordinary VM memory and VAREXT data as one atomic transaction.
    /// Unknown or undeclared records are deliberately ignored and can be retained
    /// losslessly by the runtime save adapter.
    ///
    /// # Errors
    ///
    /// Returns an error without mutation when either memory or extension data is invalid.
    pub fn prepare_runtime_state_with_extensions(
        &self,
        transaction: VmRuntimeStateTransaction,
        scope: StructuredScope,
        values: &[StructuredExtension],
    ) -> Result<(PreparedRuntimeState, BTreeSet<(u8, String)>), VmError> {
        let (structured_state, imported) = self
            .natives
            .prepare_structured_import(&transaction, scope, values)
            .map_err(VmError::InvalidState)?;
        let mut prepared = self.vm.prepare_runtime_state(transaction)?;
        prepared.structured_state = structured_state;
        Ok((prepared, imported))
    }
}

impl VmRuntimeStatePort for RuntimeVm {
    fn read_runtime_state(&self, reads: &[VmRuntimeRead]) -> Result<Vec<VmValue>, VmError> {
        self.vm.read_runtime_state(reads)
    }

    fn prepare_runtime_state(
        &self,
        transaction: VmRuntimeStateTransaction,
    ) -> Result<PreparedRuntimeState, VmError> {
        let structured_state = self
            .natives
            .prepare_structured_transaction(&transaction)
            .map_err(VmError::InvalidState)?;
        let mut prepared = self.vm.prepare_runtime_state(transaction)?;
        prepared.structured_state = structured_state;
        Ok(prepared)
    }

    fn commit_runtime_state(&mut self, prepared: PreparedRuntimeState) -> Result<(), VmError> {
        if prepared.generation != self.vm.current_generation() {
            return Err(VmError::InvalidState(
                "runtime state transaction belongs to a stale generation".into(),
            ));
        }
        if let Some(structured_state) = &prepared.structured_state {
            self.natives
                .commit_structured_state(structured_state)
                .map_err(VmError::InvalidState)?;
        }
        self.vm.commit_runtime_state(prepared)?;
        self.refresh_draw_line_string();
        Ok(())
    }
}

#[derive(Default)]
struct CaptureHost {
    first: Option<HostCallRequest>,
    overflow: Vec<HostCallRequest>,
}

impl CaptureHost {
    fn take(&mut self, request: HostRequestId) -> Option<HostCallRequest> {
        if self.first.as_ref().is_some_and(|item| item.id == request) {
            return self.first.take();
        }
        // Cooperative batches are uncommon and small. Keep the single-request hot
        // path allocation-free while retaining request-id lookup for arbitrary event
        // and completion order when several fibers reach the host together.
        let index = self.overflow.iter().position(|item| item.id == request)?;
        Some(self.overflow.remove(index))
    }

    fn is_empty(&self) -> bool {
        self.first.is_none() && self.overflow.is_empty()
    }
}

impl VmHost for CaptureHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        if self.first.is_none() {
            self.first = Some(request);
        } else {
            self.overflow.push(request);
        }
        // The runtime will classify the real wait after it has staged its own state.
        HostCallResult::Deferred
    }
}

struct CapturingRuntimeHost<'a, H> {
    immediate: &'a mut H,
    captured: CaptureHost,
}

impl<H: VmHost> VmHost for CapturingRuntimeHost<'_, H> {
    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        self.immediate.call_immediate(request)
    }

    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        self.captured.call(request)
    }
}

impl RuntimeVm {
    /// Drive with an optional immediate Host implementation. Unsupported calls still cross the
    /// ordinary caller-pumped port and retain all persistent wait/debug semantics.
    pub fn drive_with_immediate_host(
        &mut self,
        budget: RunBudget,
        mode: VmDriveMode,
        immediate: &mut impl VmHost,
    ) -> VmPortDriveReport {
        self.vm.retire_terminal_fibers();
        if matches!(mode, VmDriveMode::SelectedFiber(_)) {
            return VmPortDriveReport {
                stop: VmPortStop::DebugStopped,
                instructions: 0,
                events: Vec::new(),
            };
        }
        let mut host = CapturingRuntimeHost {
            immediate,
            captured: CaptureHost::default(),
        };
        let report = self.vm.run_slice(&mut host, &mut self.natives, budget);
        let mut events = Vec::new();
        for event in report.events {
            match event {
                crate::VmEvent::Diagnostic {
                    fiber,
                    code,
                    message,
                    origin,
                    notification,
                } => events.push(VmPortEvent::Diagnostic {
                    fiber,
                    code,
                    message,
                    origin,
                    notification,
                }),
                crate::VmEvent::HostPending { request, .. } => {
                    if let Some(request) = host.captured.take(request)
                        && let Some(definition) = self.current_host_import(request.import.key)
                    {
                        let import = HostImport {
                            import: request.import,
                            effect: definition.effect,
                            capability: definition.capability,
                            snapshot_capability: definition.snapshot_capability,
                            contract: definition.contract,
                        };
                        events.push(VmPortEvent::HostCall(VmHostRequest {
                            id: request.id,
                            fiber: request.fiber,
                            import,
                            arguments: request.arguments,
                            origin: request.origin,
                        }));
                    }
                }
                crate::VmEvent::FiberYielded { fiber } => {
                    events.push(VmPortEvent::FiberYielded(fiber));
                }
                crate::VmEvent::FiberCompleted { fiber, value } => {
                    events.push(VmPortEvent::FiberCompleted(fiber, value));
                }
                crate::VmEvent::FiberFaulted { fiber, fault } => {
                    events.push(VmPortEvent::FiberFaulted(fiber, fault));
                }
                crate::VmEvent::DebugStopped(stop) => {
                    events.push(VmPortEvent::DebugStopped(stop));
                }
            }
        }
        debug_assert!(
            host.captured.is_empty(),
            "captured host request lost its VM event"
        );
        let debug_stopped = events
            .iter()
            .any(|event| matches!(event, VmPortEvent::DebugStopped(_)));
        VmPortDriveReport {
            stop: if debug_stopped {
                VmPortStop::DebugStopped
            } else if matches!(report.stop, crate::VmRunStop::BudgetExhausted) {
                VmPortStop::BudgetExhausted
            } else {
                VmPortStop::Idle
            },
            instructions: report.instructions,
            events,
        }
    }
}

pub struct PreparedHostCompletion {
    generation: GenerationId,
    request: HostRequestId,
    completion: VmHostCompletion,
}

impl VmRuntimePort for RuntimeVm {
    type PreparedCompletion = PreparedHostCompletion;

    fn artifact_id(&self) -> Digest {
        self.vm.artifact_id()
    }

    fn current_generation(&self) -> GenerationId {
        self.vm.current_generation()
    }

    fn spawn_entry(
        &mut self,
        function: SymbolKey,
        arguments: Vec<VmValue>,
    ) -> Result<FiberId, VmError> {
        let fiber = self.vm.spawn_entry(function, arguments)?;
        // Runtime roots are dispatched sequentially by the caller-pumped system
        // controller. The newest root owns the input wait that an exact snapshot
        // must resume; older completed roots must not remain the primary fiber.
        self.vm.set_primary_fiber(fiber)?;
        Ok(fiber)
    }

    fn fiber_status(&self, fiber: FiberId) -> Option<FiberStatus> {
        self.vm.fiber_status(fiber)
    }

    fn drive(&mut self, budget: RunBudget, mode: VmDriveMode) -> VmPortDriveReport {
        self.drive_with_immediate_host(budget, mode, &mut CaptureHost::default())
    }

    fn retire_terminal_fibers(&mut self) -> usize {
        self.vm.retire_terminal_fibers()
    }

    fn validate_host_completion(
        &self,
        request: HostRequestId,
        completion: VmHostCompletion,
    ) -> Result<Self::PreparedCompletion, VmError> {
        let (fiber_id, fiber, wait) = self
            .vm
            .fibers
            .iter()
            .find_map(|(id, fiber)| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request => {
                    Some((*id, fiber, wait))
                }
                _ => None,
            })
            .ok_or(VmError::StaleHostRequest(request))?;
        let import = self
            .current_host_import(wait.import.key)
            .ok_or_else(|| VmError::InvalidState("waiting host import is missing".into()))?;
        match &completion {
            VmHostCompletion::Ready(ready) => {
                validate_ready(
                    &self.vm,
                    fiber_id,
                    fiber,
                    &import.import.name,
                    wait.result,
                    ready,
                )?;
            }
            VmHostCompletion::ReturnCurrent(_) => {
                if fiber.frames.len() <= 1 {
                    return Err(VmError::InvalidState(
                        "cannot return the root frame through a host completion".into(),
                    ));
                }
            }
            VmHostCompletion::Pending { stability, .. } => {
                // A caller-pumped runtime necessarily unwinds every external call
                // before it can ask the frontend for a service result. `may_suspend`
                // describes the EraBasic-visible operation, not this transport wait.
                if *stability == HostWaitStability::StableInput
                    && import.snapshot_capability != HostSnapshotCapability::StableWait
                {
                    return Err(VmError::InvalidState(
                        "host wait exceeds the import snapshot capability".into(),
                    ));
                }
            }
            VmHostCompletion::Error(_) => {}
        }
        Ok(PreparedHostCompletion {
            generation: self.vm.current_generation(),
            request,
            completion,
        })
    }

    fn commit_host_completion(
        &mut self,
        completion: Self::PreparedCompletion,
    ) -> Result<FiberId, VmError> {
        if completion.generation != self.vm.current_generation() {
            return Err(VmError::StaleHostRequest(completion.request));
        }
        match completion.completion {
            VmHostCompletion::Ready(ready) => self.vm.resume_host(completion.request, ready),
            VmHostCompletion::ReturnCurrent(value) => self
                .vm
                .return_current_from_host(completion.request, value.as_ref()),
            VmHostCompletion::Pending {
                stability,
                rebind_payload,
            } => {
                let (fiber_id, wait) = self
                    .vm
                    .fibers
                    .iter_mut()
                    .find_map(|(id, fiber)| match &mut fiber.state {
                        FiberState::WaitingHost(wait) if wait.request == completion.request => {
                            Some((*id, wait))
                        }
                        _ => None,
                    })
                    .ok_or(VmError::StaleHostRequest(completion.request))?;
                wait.stability = stability;
                wait.rebind_payload = rebind_payload;
                Ok(fiber_id)
            }
            VmHostCompletion::Error(message) => Err(VmError::InvalidState(format!(
                "host request failed: {message}"
            ))),
        }
    }

    fn cancel_fiber(&mut self, fiber: FiberId) -> Result<(), VmError> {
        self.vm.cancel_fiber(fiber)
    }

    fn export_era_state(&self) -> EraState {
        self.vm.export_era_state()
    }

    fn restore_era_state(&mut self, state: &EraState) -> Result<EraStateReport, VmError> {
        let report = self.vm.reset_with_era_state(state)?;
        self.refresh_draw_line_string();
        Ok(report)
    }

    fn snapshot_eligibility(&self) -> SnapshotEligibility {
        self.vm.snapshot_eligibility(&self.natives)
    }

    fn snapshot(&self) -> Result<VmSnapshot, VmError> {
        self.vm.snapshot(&self.natives)
    }

    fn encode_snapshot(&self) -> Result<Vec<u8>, VmError> {
        self.vm.encode_snapshot(&self.natives)
    }

    fn encode_unrestricted_snapshot(&self) -> Result<Vec<u8>, VmError> {
        self.vm.encode_unrestricted_snapshot(&self.natives)
    }

    fn prepare_hot_reload(&mut self, target: ValidatedArtifact) -> Result<(), VmError> {
        let migrated = self
            .natives
            .migrated_for_artifact(target.artifact())
            .map_err(VmError::Snapshot)?;
        self.vm.prepare_hot_reload_artifact(target)?;
        self.pending_natives = Some(migrated);
        Ok(())
    }

    fn commit_hot_reload(&mut self) -> Result<HotReloadReport, VmError> {
        let report = self.vm.commit_hot_reload()?;
        self.natives = self
            .pending_natives
            .take()
            .ok_or_else(|| VmError::InvalidState("prepared native migration is missing".into()))?;
        self.refresh_draw_line_string();
        Ok(report)
    }
}

impl crate::VmDebugInspect for RuntimeVm {
    fn stop_token(&self) -> Option<crate::VmStopToken> {
        self.vm.stop_token()
    }

    fn fibers(
        &self,
        stop: crate::VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugFiber>, VmError> {
        self.vm.fibers(stop, cursor, limit)
    }

    fn call_stack(
        &self,
        stop: crate::VmStopToken,
        fiber: FiberId,
    ) -> Result<Vec<crate::VmDebugFrame>, VmError> {
        self.vm.call_stack(stop, fiber)
    }

    fn operand_stack(
        &self,
        stop: crate::VmStopToken,
        fiber: FiberId,
        frame: crate::FrameId,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugOperand>, VmError> {
        self.vm.operand_stack(stop, fiber, frame, cursor, limit)
    }

    fn variables(
        &self,
        stop: crate::VmStopToken,
        cursor: Option<usize>,
        limit: usize,
    ) -> Result<crate::VmDebugPage<crate::VmDebugVariable>, VmError> {
        self.vm.variables(stop, cursor, limit)
    }

    fn read_variable(
        &self,
        stop: crate::VmStopToken,
        target: &crate::VmDebugVariableRef,
    ) -> Result<crate::VmDebugVariable, VmError> {
        crate::VmDebugInspect::read_variable(&self.vm, stop, target)
    }
}

impl crate::VmDebugControl for RuntimeVm {
    fn request_pause(&mut self) -> Result<crate::VmDebugStop, VmError> {
        self.vm.request_pause()
    }

    fn continue_execution(&mut self, stop: crate::VmStopToken) -> Result<(), VmError> {
        self.vm.continue_execution(stop)
    }

    fn step(
        &mut self,
        stop: crate::VmStopToken,
        fiber: FiberId,
        kind: crate::VmStepKind,
    ) -> Result<(), VmError> {
        self.vm.step(stop, fiber, kind)
    }

    fn write_variables(
        &mut self,
        stop: crate::VmStopToken,
        writes: &[crate::VmDebugVariableWrite],
    ) -> Result<Vec<crate::VmDebugVariable>, VmError> {
        self.vm.write_variables(stop, writes)
    }

    fn update_breakpoints(
        &mut self,
        breakpoints: &[crate::VmBreakpoint],
        remove: &[u64],
    ) -> Result<Vec<crate::VmResolvedBreakpoint>, VmError> {
        self.vm.update_breakpoints(breakpoints, remove)
    }
}

pub struct PreparedVmRestore {
    runtime: RuntimeVm,
    waits: Vec<VmWaitRebind>,
}

#[derive(Default)]
struct RestoreCaptureHost {
    waits: Vec<VmWaitRebind>,
}

impl VmHost for RestoreCaptureHost {
    fn call(&mut self, _request: HostCallRequest) -> HostCallResult {
        HostCallResult::Error("restore capture host cannot execute calls".into())
    }

    fn rebind_snapshot(&mut self, requests: &[crate::HostRebindRequest]) -> Result<(), String> {
        self.waits = requests
            .iter()
            .map(|request| VmWaitRebind {
                request: request.id,
                fiber: request.fiber,
                import: request.import.clone(),
                payload: request.payload.clone(),
            })
            .collect();
        Ok(())
    }
}

impl VmRestorePort for RuntimeVm {
    type PreparedRestore = PreparedVmRestore;

    fn prepare_restore(
        artifact: ValidatedArtifact,
        config: VmConfig,
        snapshot: VmSnapshot,
    ) -> Result<Self::PreparedRestore, VmError> {
        let mut natives = NativeServiceRegistry::for_artifact(artifact.artifact());
        let mut host = RestoreCaptureHost::default();
        let vm = Vm::restore_snapshot(artifact, config, snapshot, &mut host, &mut natives)?;
        // Preserve the captured calculated value until the runtime supplies its
        // current frontend projection after committing the restore.
        let runtime = Self {
            vm,
            natives,
            pending_natives: None,
            line_columns: DEFAULT_LINE_COLUMNS,
        };
        Ok(PreparedVmRestore {
            runtime,
            waits: host.waits,
        })
    }

    fn restore_waits(plan: &Self::PreparedRestore) -> &[VmWaitRebind] {
        &plan.waits
    }

    fn commit_restore(plan: Self::PreparedRestore) -> Result<Self, VmError> {
        Ok(plan.runtime)
    }
}

fn validate_ready(
    vm: &Vm,
    fiber_id: FiberId,
    fiber: &crate::Fiber,
    operation: &str,
    expected: Option<erabasic_bytecode::BytecodeType>,
    ready: &HostReady,
) -> Result<(), VmError> {
    let actual = ready.value.as_ref().map(VmValue::value_type);
    if expected != actual {
        return Err(VmError::InvalidArguments(format!(
            "{operation} host completion result type differs: expected {expected:?}, found {actual:?}"
        )));
    }
    for write in &ready.writes {
        if write.target.fiber.is_some_and(|owner| owner != fiber_id) {
            return Err(VmError::InvalidState(
                "host write belongs to another fiber".into(),
            ));
        }
        let definition = vm
            .artifact()
            .globals
            .iter()
            .find(|definition| definition.key == write.target.variable)
            .ok_or_else(|| VmError::InvalidState("host write variable is missing".into()))?;
        // Host completions are constructed by the trusted runtime and must update
        // reference pseudo-variables such as immutable-to-script ISTIMEOUT.
        if definition.value_type != write.value.value_type() {
            return Err(VmError::InvalidArguments(
                "host write value type differs".into(),
            ));
        }
        let _ = vm.read_place(fiber, &write.target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use erabasic_bytecode::RuntimeImport;

    fn host_request(id: u64) -> HostCallRequest {
        HostCallRequest {
            id: HostRequestId(id),
            fiber: FiberId(id.saturating_add(100)),
            import: RuntimeImport {
                key: SymbolKey([u8::try_from(id).unwrap_or(u8::MAX); 16]),
                namespace: "test".into(),
                name: format!("HOST_{id}"),
                abi_version: 1,
                parameters: Vec::new(),
                result: None,
            },
            arguments: vec![VmValue::Integer(i64::try_from(id).unwrap_or(i64::MAX))],
            origin: crate::VmExecutionOrigin {
                generation: GenerationId(1),
                function: SymbolKey([0; 16]),
                function_name: "TEST".into(),
                instruction: u32::try_from(id).unwrap_or(u32::MAX),
                command: format!("HOST_{id}"),
                source: None,
            },
        }
    }

    #[test]
    fn capture_host_keeps_the_single_request_inline() {
        let mut host = CaptureHost::default();
        let request = host_request(1);
        assert_eq!(host.call(request.clone()), HostCallResult::Deferred);
        assert!(host.overflow.is_empty());
        assert_eq!(host.take(request.id), Some(request));
        assert!(host.is_empty());
    }

    #[test]
    fn capture_host_preserves_multiple_fibers_without_fifo_assumptions() {
        let mut host = CaptureHost::default();
        let requests = [host_request(1), host_request(2), host_request(3)];
        for request in &requests {
            assert_eq!(host.call(request.clone()), HostCallResult::Deferred);
        }
        assert_eq!(host.take(requests[1].id), Some(requests[1].clone()));
        assert_eq!(host.take(requests[0].id), Some(requests[0].clone()));
        assert_eq!(host.take(requests[2].id), Some(requests[2].clone()));
        assert!(host.is_empty());
    }
}
