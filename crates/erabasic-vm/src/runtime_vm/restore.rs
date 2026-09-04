#[allow(clippy::wildcard_imports)]
use super::*;
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
            candidate_base_column_stamp: CandidateColumnBase::Unforked,
            candidate_base_array_stamp: None,
            line_columns: DEFAULT_LINE_COLUMNS,
            pending_completion_events: Vec::new(),
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

pub(super) fn validate_ready(
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
        // Public Host descriptors never carry the VM-private backing capability.
        // A legitimate REF write names its live formal and resolves the binding in VM.
        if write.target.backing.is_some() {
            return Err(VmError::InvalidState(
                "Host cannot inject an array backing identity".into(),
            ));
        }
        let (_, definition) = vm.place_definition(fiber, &write.target).map_err(|error| {
            VmError::ScriptFailure(crate::ExecutionFailure::classified(
                crate::FaultCategory::HostContract,
                crate::VmFaultCode::Host,
                error.to_string(),
            ))
        })?;
        // Host completions are constructed by the trusted runtime and must update
        // reference pseudo-variables such as immutable-to-script ISTIMEOUT.
        if definition.value_type != write.value.value_type() {
            return Err(VmError::InvalidArguments(
                "host write value type differs".into(),
            ));
        }
        let _ = vm.read_place(fiber, &write.target).map_err(|error| {
            VmError::ScriptFailure(crate::ExecutionFailure::classified(
                crate::FaultCategory::HostContract,
                crate::VmFaultCode::Host,
                error.to_string(),
            ))
        })?;
    }
    Ok(())
}
