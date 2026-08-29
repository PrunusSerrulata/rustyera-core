//! Script input controls and negotiated observations share the session owner.
#[allow(clippy::wildcard_imports)]
use super::*;
impl RuntimeSession {
    fn emit_input_notice_once(
        &mut self,
        vm: &RuntimeVm,
        request: &VmHostRequest,
        code: &str,
        message: &str,
    ) -> Result<(), RuntimeError> {
        let site = (
            code.to_owned(),
            request.origin.generation.0,
            request.origin.function,
            request.origin.instruction,
        );
        if self.input_notice_sites.insert(site) {
            self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    context: self.vm_diagnostic_context(vm, &request.origin, code),
                    code: code.into(),
                    level: RuntimeLogLevel::Warning,
                    message: message.into(),
                    source: protocol_execution_origin(request.origin.clone()).source,
                    notification: DiagnosticNotification::default(),
                }),
                None,
            )?;
        }
        Ok(())
    }

    pub(in crate::session) fn dispatch_input_extensions(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
    ) -> Result<bool, RuntimeError> {
        if !matches!(
            name,
            "__GETKEY_ACTIVE"
                | "GETKEY"
                | "GETKEYTRIGGERED"
                | "SEQUENCEINPUT"
                | "DISABLE_INPUT_MACRO"
                | "ENABLE_INPUT_MACRO"
                | "ENV_HAS_CAPABILITY"
                | "GETPLATFORM"
        ) {
            return Ok(false);
        }
        if !vm
            .vm()
            .artifact()
            .manifest
            .compatibility
            .supports_snake_input()
        {
            if matches!(name, "GETKEY" | "GETKEYTRIGGERED") {
                return Ok(false);
            }
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Error(erabasic_vm::ExecutionFailure::classified(
                    erabasic_vm::FaultCategory::Permission,
                    erabasic_vm::VmFaultCode::Host,
                    "snake input API used by an incompatible artifact",
                )),
            )?;
            return Ok(true);
        }
        let value = self.execute_input_extension(vm, request, name)?;
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Ready(HostReady {
                value: Some(VmValue::Integer(value)),
                writes: Vec::new(),
            }),
        )?;
        Ok(true)
    }

    fn execute_input_extension(
        &mut self,
        vm: &RuntimeVm,
        request: &VmHostRequest,
        name: &str,
    ) -> Result<i64, RuntimeError> {
        let value = match (name, request.arguments.as_slice()) {
            ("__GETKEY_ACTIVE", []) => {
                if self.client_focused && !self.environment.has(INPUT_DEVICE_LATCH_CAPABILITY, 1) {
                    self.emit_input_notice_once(
                        vm,
                        request,
                        "compat.input.device_latch_unavailable",
                        "this client does not provide physical key and mouse latch state; GETKEY APIs return 0",
                    )?;
                }
                i64::from(
                    self.client_focused && self.environment.has(INPUT_DEVICE_LATCH_CAPABILITY, 1),
                )
            }
            ("GETKEY" | "GETKEYTRIGGERED", [VmValue::Integer(code)]) => {
                // The compiler/form continuation already performed the active gate
                // before evaluating code. A focus change in that argument must not
                // introduce a second gate or change its source execution order.
                if self.environment.has(INPUT_DEVICE_LATCH_CAPABILITY, 1) {
                    self.device_input
                        .snake_query(*code, name == "GETKEYTRIGGERED")
                } else {
                    0
                }
            }
            ("SEQUENCEINPUT", [VmValue::String(text)]) => {
                crate::input_set::ensure_size(text.len()).map_err(RuntimeError::ResourceLimit)?;
                self.input_controller.pending_sequence = Some(PendingSequence {
                    text: text.clone(),
                    site: SequenceSite {
                        artifact: vm.artifact_id(),
                        function: request.origin.function,
                        instruction: request.origin.instruction,
                    },
                });
                0
            }
            ("DISABLE_INPUT_MACRO", []) => {
                self.input_controller.macro_enabled = false;
                0
            }
            ("ENABLE_INPUT_MACRO", []) => {
                self.input_controller.macro_enabled = true;
                0
            }
            ("ENV_HAS_CAPABILITY", [VmValue::String(name)]) => {
                i64::from(self.environment.has(name, 1))
            }
            ("ENV_HAS_CAPABILITY", [VmValue::String(name), VmValue::Integer(major)]) => {
                i64::from(self.environment.has(name, *major))
            }
            ("GETPLATFORM", []) => {
                self.emit_input_notice_once(
                    vm,
                    request,
                    "compat.portability.platform_mapping",
                    "GETPLATFORM reports NF timed viewport compatibility (0/5), not the host operating system",
                )?;
                if self.environment.has(INPUT_TIMED_VIEWPORT_CAPABILITY, 1) {
                    0
                } else {
                    5
                }
            }
            _ => {
                return Err(RuntimeError::Internal(
                    "input Host call physical signature differs".into(),
                ));
            }
        };
        Ok(value)
    }
}
