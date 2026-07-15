use erabasic_bytecode::{Digest, HostSnapshotCapability, SymbolKey};
use erabasic_validator::ValidatedArtifact;

use crate::{
    EraState, EraStateReport, FiberId, FiberState, FiberStatus, GenerationId, HostCallRequest,
    HostCallResult, HostReady, HostRequestId, HostWaitStability, HotReloadReport,
    NativeServiceRegistry, PreparedRuntimeState, RunBudget, SnapshotEligibility, Vm, VmConfig,
    VmDriveMode, VmError, VmHost, VmHostCompletion, VmHostRequest, VmPortDriveReport, VmPortEvent,
    VmPortStop, VmRuntimePort, VmRuntimeRead, VmRuntimeStatePort, VmRuntimeStateTransaction,
    VmSnapshot, VmValue,
};

/// Runtime-facing VM owner. It keeps native services beside the interpreter so the
/// caller-pumped runtime port never needs a callback parameter.
pub struct RuntimeVm {
    vm: Vm,
    natives: NativeServiceRegistry,
}

impl RuntimeVm {
    #[must_use]
    pub fn new(artifact: ValidatedArtifact, config: VmConfig) -> Self {
        let natives = NativeServiceRegistry::for_artifact(artifact.artifact());
        Self {
            vm: Vm::new(artifact, config),
            natives,
        }
    }

    #[must_use]
    pub fn new_with_seed(artifact: ValidatedArtifact, config: VmConfig, seed: u64) -> Self {
        let natives = NativeServiceRegistry::for_artifact_with_seed(artifact.artifact(), seed);
        Self {
            vm: Vm::new(artifact, config),
            natives,
        }
    }

    #[must_use]
    pub const fn vm(&self) -> &Vm {
        &self.vm
    }

    pub const fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    /// Whether at least one fiber can make progress without a host completion.
    #[must_use]
    pub fn has_runnable_fibers(&self) -> bool {
        self.vm
            .fibers
            .values()
            .any(|fiber| matches!(fiber.state, FiberState::Runnable))
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
        self.vm.prepare_runtime_state(transaction)
    }

    fn commit_runtime_state(&mut self, prepared: PreparedRuntimeState) -> Result<(), VmError> {
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
            }
        }
        VmPortDriveReport {
            stop: if matches!(report.stop, crate::VmRunStop::BudgetExhausted) {
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
        self.vm.prepare_hot_reload_artifact(target).map(|_| ())
    }

    fn commit_hot_reload(&mut self) -> Result<HotReloadReport, VmError> {
        let report = self.vm.commit_hot_reload()?;
        self.natives = NativeServiceRegistry::for_artifact(self.vm.artifact());
        Ok(report)
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
