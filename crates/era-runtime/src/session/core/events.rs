// This is part of the split RuntimeSession implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in crate::session) fn vm_diagnostic_context(
        &self,
        vm: &RuntimeVm,
        origin: &erabasic_vm::VmExecutionOrigin,
        code: &str,
    ) -> Option<Box<era_runtime_protocol::CompatibilityDiagnosticContext>> {
        if !code.starts_with("compat.") {
            return None;
        }
        Some(Box::new(
            era_runtime_protocol::CompatibilityDiagnosticContext {
                identity: Some(vm.vm().artifact().manifest.compatibility.clone()),
                stage: "runtime".into(),
                api: Some(origin.command.clone()),
                required_capability: None,
                // Old live frames retain their generation; never stamp them with the new artifact.
                artifact: (origin.generation == vm.vm().current_generation())
                    .then(|| ProtocolBytes::new(vm.artifact_id().0.to_vec())),
                project_load_id: Some(self.project_load_id),
                runtime_epoch: Some(self.epoch.0),
                generation: Some(origin.generation.0),
            },
        ))
    }

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
                notification,
                ..
            } => self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    context: self.vm_diagnostic_context(vm, &origin, &code),
                    code,
                    level: RuntimeLogLevel::Warning,
                    message,
                    source: protocol_execution_origin(origin).source,
                    notification: protocol_diagnostic_notification(notification),
                }),
                None,
            ),
            VmPortEvent::HostCall(request) => self.handle_host_call(vm, &request),
            VmPortEvent::FiberFaulted(_, fault) => self.fault_from_vm(&fault),
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
