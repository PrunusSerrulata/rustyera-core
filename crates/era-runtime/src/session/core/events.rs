// This is part of the split RuntimeSession implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn handle_vm_event(
        &mut self,
        vm: &mut RuntimeVm,
        event: VmPortEvent,
    ) -> Result<(), RuntimeError> {
        match event {
            VmPortEvent::Diagnostic {
                code,
                message,
                origin,
                ..
            } => self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    code,
                    level: RuntimeLogLevel::Warning,
                    message,
                    source: protocol_execution_origin(origin).source,
                }),
                None,
            ),
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
                    if self.controller.is_complete() && self.controller.deferred_flow.is_some() {
                        if self.controller.flow == Some(SystemFlow::Shop)
                            && self.controller.step == SystemStep::ShopEvent
                        {
                            return self.continue_system_flow(vm);
                        }
                        let flow = self
                            .controller
                            .deferred_flow
                            .take()
                            .expect("checked deferred flow");
                        self.controller.clear();
                        self.controller.flow = Some(flow);
                        return self.begin_flow(vm, flow);
                    }
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
}
