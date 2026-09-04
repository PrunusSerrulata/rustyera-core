#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_presentation_text_and_style(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_presentation_text_and_bars(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_debug_clear(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_presentation_text_and_bars(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name.as_str(), "BAR" | "BARL") {
            *status = HostDispatchStatus::Handled;
            let value = integer_argument_value(request, 0)?;
            let maximum = integer_argument_value(request, 1)?;
            let length = integer_argument_value(request, 2)?;
            let replace = &vm.vm().artifact().project_data.static_data.replace;
            let bar = match make_bar(
                value,
                maximum,
                length,
                replace.bar_char_1,
                replace.bar_char_2,
            ) {
                Ok(value) => value,
                Err(message) => {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        message,
                    );
                }
            };
            self.presentation
                .append_print_text(bar, false, name == "BARL");
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_presentation();
        }
        if matches!(
            name.as_str(),
            "DEBUGPRINT" | "DEBUGPRINTL" | "DEBUGPRINTFORM" | "DEBUGPRINTFORML"
        ) {
            *status = HostDispatchStatus::Handled;
            let mut appended = String::new();
            for value in &request.arguments {
                appended.push_str(&display_value(value));
            }
            if name.ends_with('L') {
                appended.push_str("\r\n");
            }
            let cursor = self
                .debug_output_base
                .saturating_add(u64::try_from(self.debug_output.len()).unwrap_or(u64::MAX));
            self.debug_output.push_str(&appended);
            if self.debug_output.len() > 1_048_576 {
                let remove = self.debug_output.len() - 1_048_576;
                let boundary = self
                    .debug_output
                    .char_indices()
                    .find_map(|(index, _)| (index >= remove).then_some(index))
                    .unwrap_or(remove);
                self.debug_output.drain(..boundary);
                self.debug_output_base = self
                    .debug_output_base
                    .saturating_add(u64::try_from(boundary).unwrap_or(u64::MAX));
            }
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            if self.debug_output_subscribed {
                self.emit_debug(
                    DebugMessage::Response(DebugResponse::ScriptOutput(ScriptOutputChunk {
                        cursor,
                        next_cursor: cursor
                            .saturating_add(u64::try_from(appended.len()).unwrap_or(u64::MAX)),
                        text: appended,
                        truncated: false,
                    })),
                    None,
                )?;
            }
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_presentation_debug_clear(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "DEBUGCLEAR" {
            *status = HostDispatchStatus::Handled;
            self.debug_output_base = self
                .debug_output_base
                .saturating_add(u64::try_from(self.debug_output.len()).unwrap_or(u64::MAX));
            self.debug_output.clear();
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        match PreparedPresentationState::prepare(name, &request.arguments) {
            Ok(Some(prepared)) => {
                *status = HostDispatchStatus::Handled;
                prepared.apply(&mut self.presentation);
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            Ok(None) => {}
            Err(PresentationStatePreparationError::Alignment) => {
                *status = HostDispatchStatus::Handled;
                if matches!(request.argument(0), Some(VmValue::String(_))) {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        "ALIGNMENT expects LEFT, CENTER, or RIGHT",
                    );
                }
                return self.fault(
                    FaultCode::VmFault,
                    "ALIGNMENT expects LEFT, CENTER, or RIGHT",
                    Some(request.origin.clone()),
                );
            }
            Err(PresentationStatePreparationError::FontStyle(error)) => {
                *status = HostDispatchStatus::Handled;
                return Err(error);
            }
            Err(PresentationStatePreparationError::Color(error)) => {
                *status = HostDispatchStatus::Handled;
                if matches!(
                    request.arguments.as_slice(),
                    [
                        VmValue::Integer(_),
                        VmValue::Integer(_),
                        VmValue::Integer(_)
                    ]
                ) {
                    return complete_script_fault(
                        vm,
                        request,
                        erabasic_vm::ScriptFaultKind::Argument,
                        error,
                    );
                }
                return self.fault(FaultCode::VmFault, error, Some(request.origin.clone()));
            }
        }
        Ok(())
    }
}
