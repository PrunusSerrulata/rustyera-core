#[allow(clippy::wildcard_imports)]
use super::*;
#[derive(Default)]
pub(super) struct CaptureHost {
    first: Option<HostCallRequest>,
    pub(super) overflow: Vec<HostCallRequest>,
}

impl CaptureHost {
    pub(super) fn take(&mut self, request: HostRequestId) -> Option<HostCallRequest> {
        if self.first.as_ref().is_some_and(|item| item.id == request) {
            return self.first.take();
        }
        // Cooperative batches are uncommon and small. Keep the single-request hot
        // path allocation-free while retaining request-id lookup for arbitrary event
        // and completion order when several fibers reach the host together.
        let index = self.overflow.iter().position(|item| item.id == request)?;
        Some(self.overflow.remove(index))
    }

    pub(super) fn is_empty(&self) -> bool {
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
    fn path_memo_safe(&self, import: &erabasic_bytecode::RuntimeImport) -> bool {
        self.immediate.path_memo_safe(import)
    }

    fn call_immediate(&mut self, request: ImmediateHostCall<'_>) -> ImmediateHostCallResult {
        self.immediate.call_immediate(request)
    }

    fn call(&mut self, request: HostCallRequest) -> HostCallResult {
        self.captured.call(request)
    }
}

impl RuntimeVm {
    fn deliver_captured_host(&mut self, request: HostCallRequest, events: &mut Vec<VmPortEvent>) {
        let definition = self
            .vm
            .fibers
            .get(&request.fiber)
            .and_then(|fiber| match &fiber.state {
                FiberState::WaitingHost(wait) if wait.request == request.id => {
                    self.waiting_host_import(request.fiber, wait)
                }
                _ => None,
            });
        if let Some(import) = definition.filter(|definition| definition.import == request.import) {
            events.push(VmPortEvent::HostCall(VmHostRequest {
                id: request.id,
                fiber: request.fiber,
                import,
                arguments: request.arguments,
                omitted_arguments: request.omitted_arguments,
                origin: request.origin,
            }));
        } else {
            // Missing owner/grant is an invariant failure, never a name fallback
            // or a silently dropped HostPending event.
            let failure = crate::ExecutionFailure::classified(
                crate::FaultCategory::InternalInvariant,
                crate::VmFaultCode::Host,
                "captured Host request lost its exact generation/owner authorization",
            );
            if let Ok((fiber, Some(fault))) = self.vm.fail_waiting_host(request.id, failure) {
                events.push(VmPortEvent::FiberFaulted(fiber, fault));
            } else {
                // Internal failures are never catchable. A missing wait or
                // an impossible recovery still reports the captured origin.
                let fault = crate::VmFault::from_origin(
                    request.fiber,
                    request.origin,
                    crate::ExecutionFailure::classified(
                        crate::FaultCategory::InternalInvariant,
                        crate::VmFaultCode::Host,
                        "captured Host request has no recoverable authorized owner",
                    ),
                );
                let published = match self.vm.fibers.remove(&request.fiber) {
                    Some(mut fiber) => {
                        let published = match self.vm.transition_fault(&mut fiber, fault) {
                            crate::interpreter::fault_hooks::FaultTransition::Published(fault) => {
                                *fault
                            }
                            crate::interpreter::fault_hooks::FaultTransition::HookStarted => {
                                unreachable!("internal invariant faults cannot start script hooks")
                            }
                        };
                        self.vm.fibers.insert(request.fiber, fiber);
                        published
                    }
                    None => fault,
                };
                events.push(VmPortEvent::FiberFaulted(request.fiber, published));
            }
        }
    }

    /// Drive with an optional immediate Host implementation. Unsupported calls still cross the
    /// ordinary caller-pumped port and retain all persistent wait/debug semantics.
    pub fn drive_with_immediate_host(
        &mut self,
        budget: RunBudget,
        mode: VmDriveMode,
        immediate: &mut impl VmHost,
    ) -> VmPortDriveReport {
        if !self.pending_completion_events.is_empty() {
            return VmPortDriveReport {
                stop: VmPortStop::Idle,
                instructions: 0,
                events: std::mem::take(&mut self.pending_completion_events),
            };
        }
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
                    if let Some(request) = host.captured.take(request) {
                        self.deliver_captured_host(request, &mut events);
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
        if self.has_pending_events() {
            // The caller has not observed the terminal state yet. Retiring it can
            // otherwise discard the primary identity before event dispatch.
            0
        } else {
            self.vm.retire_terminal_fibers()
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
            .waiting_host_import(fiber_id, wait)
            .ok_or_else(|| VmError::InvalidState("waiting host import is missing".into()))?;
        match &completion {
            VmHostCompletion::Ready(ready) => {
                super::restore::validate_ready(
                    &self.vm,
                    fiber_id,
                    fiber,
                    &import.import.name,
                    wait.result,
                    ready,
                )?;
            }
            VmHostCompletion::ReturnCurrent(_) => {
                if wait.form_scope.is_some() {
                    return Err(VmError::InvalidState(
                        "direct Host expression cannot return its owner frame".into(),
                    ));
                }
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
            VmHostCompletion::ReturnCurrent(value) => {
                let fiber = self
                    .vm
                    .return_current_from_host(completion.request, value.as_ref())?;
                self.natives
                    .retain_map_leases(&crate::interpreter::map_calls::live_map_leases(
                        self.vm.fibers.values(),
                    ))
                    .map_err(|error| VmError::InvalidState(error.to_string()))?;
                if let Some(FiberStatus::Completed(value)) = self.vm.fiber_status(fiber) {
                    self.pending_completion_events
                        .push(VmPortEvent::FiberCompleted(fiber, value));
                }
                Ok(fiber)
            }
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
            VmHostCompletion::Error(failure) => {
                let (fiber, fault) = self.vm.fail_waiting_host(completion.request, failure)?;
                self.natives
                    .retain_map_leases(&crate::interpreter::map_calls::live_map_leases(
                        self.vm.fibers.values(),
                    ))
                    .map_err(|error| VmError::InvalidState(error.to_string()))?;
                if let Some(fault) = fault {
                    self.pending_completion_events
                        .push(VmPortEvent::FiberFaulted(fiber, fault));
                }
                Ok(fiber)
            }
        }
    }

    fn cancel_fiber(&mut self, fiber: FiberId) -> Result<(), VmError> {
        self.vm.cancel_fiber(fiber)?;
        if let Some(FiberStatus::Faulted(fault)) = self.vm.fiber_status(fiber) {
            self.pending_completion_events
                .push(VmPortEvent::FiberFaulted(fiber, fault));
        }
        self.natives
            .retain_map_leases(&crate::interpreter::map_calls::live_map_leases(
                self.vm.fibers.values(),
            ))
            .map_err(|error| VmError::InvalidState(error.to_string()))
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
        if !self.pending_completion_events.is_empty() {
            return SnapshotEligibility::Ineligible(vec![
                crate::SnapshotBlocker::PendingCompletionEvents,
            ]);
        }
        self.vm.snapshot_eligibility(&self.natives)
    }

    fn snapshot(&self) -> Result<VmSnapshot, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::Snapshot(
                "host completion events have not been delivered".into(),
            ));
        }
        self.vm.snapshot(&self.natives)
    }

    fn encode_snapshot(&self) -> Result<Vec<u8>, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::Snapshot(
                "host completion events have not been delivered".into(),
            ));
        }
        self.vm.encode_snapshot(&self.natives)
    }

    fn encode_unrestricted_snapshot(&self) -> Result<Vec<u8>, VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::Snapshot(
                "host completion events have not been delivered".into(),
            ));
        }
        self.vm.encode_unrestricted_snapshot(&self.natives)
    }

    fn prepare_hot_reload(&mut self, target: ValidatedArtifact) -> Result<(), VmError> {
        if !self.pending_completion_events.is_empty() {
            return Err(VmError::HotReload(
                "host completion events have not been delivered".into(),
            ));
        }
        let base_column_stamp = self
            .natives
            .column_identity_stamp()
            .map_err(VmError::Snapshot)?;
        let migrated = self
            .natives
            .migrated_for_artifact(target.artifact())
            .map_err(VmError::Snapshot)?;
        self.vm.prepare_hot_reload_artifact(target)?;
        let map_stamp = self.natives.map_lease_stamp().map_err(VmError::Snapshot)?;
        self.pending_natives = Some((migrated, base_column_stamp, map_stamp));
        Ok(())
    }

    fn commit_hot_reload(&mut self) -> Result<HotReloadReport, VmError> {
        let (_, base_column_stamp, base_map_stamp) = self
            .pending_natives
            .as_ref()
            .ok_or_else(|| VmError::InvalidState("prepared native migration is missing".into()))?;
        self.natives
            .validate_column_identity_stamp(*base_column_stamp)
            .map_err(VmError::InvalidState)?;
        self.natives
            .validate_map_lease_stamp(*base_map_stamp)
            .map_err(VmError::InvalidState)?;
        let report = self.vm.commit_hot_reload()?;
        let (natives, _, _) = self
            .pending_natives
            .take()
            .expect("validated native migration remains available");
        self.natives = natives;
        self.refresh_draw_line_string();
        Ok(report)
    }
}
