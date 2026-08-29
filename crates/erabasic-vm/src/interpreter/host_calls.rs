//! One ordinary Host boundary for static bytecode and direct runtime expressions.
use super::{StepError, StepOutcome};
use crate::{
    Fiber, FiberState, HostCallRequest, HostCallResult, HostWaitStability, Vm, VmFaultCode, VmHost,
    VmValue, WaitingHost,
};
use erabasic_bytecode::HostSnapshotCapability;

impl Vm {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch_host_call(
        &mut self,
        fiber: &mut Fiber,
        target: erabasic_bytecode::HostImport,
        arguments: Vec<VmValue>,
        omitted_arguments: Vec<usize>,
        origin: crate::VmExecutionOrigin,
        form_scope: Option<crate::RuntimeHostScope>,
        host: &mut impl VmHost,
        host_calls: &mut u32,
    ) -> Result<StepOutcome, StepError> {
        if omitted_arguments.windows(2).any(|pair| pair[0] >= pair[1])
            || omitted_arguments
                .iter()
                .any(|index| !matches!(arguments.get(*index), Some(VmValue::Integer(i64::MIN))))
        {
            return Err(StepError::classified(
                crate::FaultCategory::InternalInvariant,
                VmFaultCode::Host,
                "Host omission metadata differs from its physical arguments",
            ));
        }
        // The caller checks its slice budget before consuming arguments or issuing effects.
        self.invalidate_path_memo(fiber.id);
        let request = self.allocate_request_id();
        *host_calls = host_calls.saturating_add(1);
        match host.call(HostCallRequest {
            id: request,
            fiber: fiber.id,
            import: target.import.clone(),
            arguments,
            omitted_arguments,
            origin: origin.clone(),
        }) {
            HostCallResult::Ready(ready) => {
                self.apply_host_ready(fiber, target.import.result, ready)
                    .map_err(|error| {
                        StepError::classified(
                            crate::FaultCategory::HostContract,
                            VmFaultCode::Host,
                            error.to_string(),
                        )
                    })?;
                Ok(StepOutcome::Continue)
            }
            HostCallResult::Pending {
                stability,
                rebind_payload,
            } => {
                if !target.effect.may_suspend
                    || stability == HostWaitStability::StableInput
                        && target.snapshot_capability != HostSnapshotCapability::StableWait
                {
                    return Err(StepError::classified(
                        crate::FaultCategory::HostContract,
                        VmFaultCode::Host,
                        "Host returned a wait above its authorized capability",
                    ));
                }
                fiber.state = FiberState::WaitingHost(WaitingHost {
                    request,
                    result: target.import.result,
                    import: target.import,
                    stability,
                    rebind_payload,
                    origin,
                    form_scope,
                });
                Ok(StepOutcome::Blocked)
            }
            HostCallResult::Error(error) => Err(error),
            HostCallResult::Deferred => {
                // Transport capture is not a claim that a source function may suspend.
                fiber.state = FiberState::WaitingHost(WaitingHost {
                    request,
                    result: target.import.result,
                    import: target.import,
                    stability: HostWaitStability::Transient,
                    rebind_payload: Vec::new(),
                    origin,
                    form_scope,
                });
                Ok(StepOutcome::Blocked)
            }
        }
    }
}
