use erabasic_bytecode::{Digest, HostSnapshotCapability, SymbolKey};
use erabasic_validator::ValidatedArtifact;

use crate::structured::{StructuredExtension, StructuredScope};
use crate::{
    EraState, EraStateReport, FiberId, FiberState, FiberStatus, GenerationId, HostCallRequest,
    HostCallResult, HostReady, HostRequestId, HostWaitStability, HotReloadReport,
    NativeServiceRegistry, PreparedRuntimeState, RunBudget, SnapshotEligibility, Vm, VmConfig,
    VmDriveMode, VmError, VmHost, VmHostCompletion, VmHostRequest, VmPortDriveReport, VmPortEvent,
    VmPortStop, VmRestorePort, VmRuntimePort, VmRuntimeRead, VmRuntimeStatePort,
    VmRuntimeStateTransaction, VmSnapshot, VmValue, VmWaitRebind,
};
use std::collections::BTreeSet;

use crate::debug::DebugState;

/// Runtime-facing VM owner. It keeps native services beside the interpreter so the
/// caller-pumped runtime port never needs a callback parameter.
pub struct RuntimeVm {
    vm: Vm,
    natives: NativeServiceRegistry,
    pending_natives: Option<NativeServiceRegistry>,
}

/// Opaque candidate state prepared against one exact artifact generation.
/// It intentionally excludes fibers, frames and scheduler counters.
pub struct PreparedCandidateState {
    artifact_id: Digest,
    memory: crate::Memory,
    natives: NativeServiceRegistry,
}

impl RuntimeVm {
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
        vm.pending_reload = None;
        vm.debug = DebugState::default();
        let natives = self
            .natives
            .fork_for_artifact(vm.artifact())
            .map_err(VmError::Snapshot)?;
        Ok(Self {
            vm,
            natives,
            pending_natives: None,
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
        Ok(())
    }

    #[must_use]
    pub fn new(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        let natives = NativeServiceRegistry::for_artifact(artifact.artifact());
        Self {
            vm: Vm::new(artifact, config),
            natives,
            pending_natives: None,
        }
    }

    #[must_use]
    pub fn new_with_seed(artifact: ValidatedArtifact, config: VmConfig, seed: u64) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        Self {
            vm: Vm::new(artifact, config),
            natives,
            pending_natives: None,
        }
    }

    #[must_use]
    pub const fn vm(&self) -> &Vm {
        &self.vm
    }

    pub const fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    /// Return the active dimensions for a named global, local, or bound
    /// reference variable in the requesting fiber.
    #[must_use]
    pub fn variable_dimensions(&self, fiber: FiberId, name: &str) -> Option<Vec<u64>> {
        self.vm.variable_dimensions(fiber, name)
    }

    /// Whether at least one fiber can make progress without a host completion.
    #[must_use]
    pub fn has_runnable_fibers(&self) -> bool {
        self.vm
            .fibers
            .values()
            .any(|fiber| matches!(fiber.state, FiberState::Runnable))
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
        self.vm.commit_runtime_state(prepared)
    }
}

struct CaptureHost {
    requests: Vec<HostCallRequest>,
}

impl VmHost for CaptureHost {
    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        self.requests.push(request);
        // The runtime will classify the real wait after it has staged its own state.
        HostCallResult::Deferred
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
        self.vm.spawn_entry(function, arguments)
    }

    fn fiber_status(&self, fiber: FiberId) -> Option<FiberStatus> {
        self.vm.fiber_status(fiber)
    }

    fn drive(&mut self, budget: RunBudget, mode: VmDriveMode) -> VmPortDriveReport {
        if matches!(mode, VmDriveMode::SelectedFiber(_)) {
            return VmPortDriveReport {
                stop: VmPortStop::DebugStopped,
                instructions: 0,
                events: Vec::new(),
            };
        }
        let mut host = CaptureHost {
            requests: Vec::new(),
        };
        let report = self.vm.run_slice(&mut host, &mut self.natives, budget);
        let mut requests = host
            .requests
            .into_iter()
            .map(|request| (request.id, request))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut events = Vec::new();
        for event in report.events {
            match event {
                crate::VmEvent::HostPending { request, .. } => {
                    if let Some(request) = requests.remove(&request) {
                        let import = self
                            .vm
                            .artifact()
                            .host_imports
                            .iter()
                            .find(|candidate| candidate.import.key == request.import.key)
                            .cloned();
                        if let Some(import) = import {
                            events.push(VmPortEvent::HostCall(VmHostRequest {
                                id: request.id,
                                fiber: request.fiber,
                                import,
                                arguments: request.arguments,
                                origin: request.origin,
                            }));
                        }
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
            .vm
            .artifact()
            .host_imports
            .iter()
            .find(|candidate| candidate.import.key == wait.import.key)
            .ok_or_else(|| VmError::InvalidState("waiting host import is missing".into()))?;
        match &completion {
            VmHostCompletion::Ready(ready) => {
                validate_ready(&self.vm, fiber_id, fiber, wait.result, ready)?;
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
        self.vm.reset_with_era_state(state)
    }

    fn snapshot_eligibility(&self) -> SnapshotEligibility {
        self.vm.snapshot_eligibility(&self.natives)
    }

    fn snapshot(&self) -> Result<VmSnapshot, VmError> {
        self.vm.snapshot(&self.natives)
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
        Ok(PreparedVmRestore {
            runtime: Self {
                vm,
                natives,
                pending_natives: None,
            },
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
    expected: Option<erabasic_bytecode::BytecodeType>,
    ready: &HostReady,
) -> Result<(), VmError> {
    if expected != ready.value.as_ref().map(VmValue::value_type) {
        return Err(VmError::InvalidArguments(
            "host completion result type differs".into(),
        ));
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
