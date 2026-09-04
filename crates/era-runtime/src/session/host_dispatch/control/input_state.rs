#[allow(clippy::needless_borrow, clippy::single_match_else)]
impl RuntimeSession {
    fn dispatch_control_input_state(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_control_text_box(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_bitmap_and_hotkeys(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_flow_input(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_control_text_box(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GETTEXTBOX" {
            *status = HostDispatchStatus::Handled;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(self.text_box.clone())),
                    writes: Vec::new(),
                }),
            );
        }
        if name == "SETTEXTBOX" {
            *status = HostDispatchStatus::Handled;
            string_argument_value(request, 0, &name)?.clone_into(&mut self.text_box);
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_projection_state();
        }
        if name == "CLEARTEXTBOX" {
            *status = HostDispatchStatus::Handled;
            self.text_box.clear();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_projection_state();
        }
        if matches!(name.as_str(), "MOVETEXTBOX" | "RESUMETEXTBOX") {
            *status = HostDispatchStatus::Handled;
            self.text_box_layout = if name == "MOVETEXTBOX" {
                TextBoxLayout {
                    x: integer_argument_value(request, 0)?,
                    y: integer_argument_value(request, 1)?,
                    width: integer_argument_value(request, 2)?,
                }
            } else {
                TextBoxLayout::default()
            };
            commit_integer_result(vm, request.id, 1)?;
            return self.emit_projection_state();
        }
        Ok(())
    }

    fn dispatch_control_bitmap_and_hotkeys(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "BITMAP_CACHE_ENABLE" {
            *status = HostDispatchStatus::Handled;
            // Reference compatibility no-op: bitmap line caching is only a
            // renderer performance hint and cannot affect portable semantics.
            let snake = self.project_snapshot.as_ref().is_some_and(|project| {
                project
                    .manifest
                    .compatibility
                    .supports_snake_display_state()
            });
            if snake && !self.bitmap_cache_notice_emitted {
                let code = "compat.bitmap_cache_enable_noop";
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        context: self.vm_diagnostic_context(vm, &request.origin, code),
                        code: code.into(),
                        level: RuntimeLogLevel::Warning,
                        message: "BITMAP_CACHE_ENABLE is accepted as a compatibility no-op; RustyEra does not expose renderer bitmap-cache policy"
                            .into(),
                        source: protocol_execution_origin(request.origin.clone()).source,
                        notification: DiagnosticNotification::LogOnly,
                    }),
                    None,
                )?;
                self.bitmap_cache_notice_emitted = true;
            }
            return if snake {
                commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))
            } else {
                commit_integer_result(vm, request.id, 0)
            };
        }
        if name == "HOTKEY_STATE_INIT" {
            *status = HostDispatchStatus::Handled;
            let raw_size = integer_argument_value(request, 0)?;
            if raw_size < 0 {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HOTKEY_STATE_INIT size must be non-negative",
                );
            }
            let size = usize::try_from(raw_size).map_err(|_| {
                RuntimeError::Internal("HOTKEY_STATE_INIT size must be non-negative".into())
            })?;
            self.hotkey_state = vec![0; size];
            commit_integer_result(vm, request.id, 0)?;
            return self.emit_projection_state();
        }
        if name == "HOTKEY_STATE" {
            *status = HostDispatchStatus::Handled;
            let Ok(index) = usize::try_from(integer_argument_value(request, 0)?) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HOTKEY_STATE index must be non-negative",
                );
            };
            if request.arguments.len() < 2 {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "HOTKEY_STATE dereferenced its absent second source argument",
                );
            }
            let value = integer_argument_value(request, 1)?;
            let Some(slot) = self.hotkey_state.get_mut(index) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "HOTKEY_STATE requires an initialized in-range index",
                );
            };
            *slot = value;
            commit_integer_result(vm, request.id, 0)?;
            return self.emit_projection_state();
        }
        Ok(())
    }

    fn dispatch_control_flow_input(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &String,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "FLOWINPUT" {
            *status = HostDispatchStatus::Handled;
            self.flow_input_default = integer_argument_value(request, 0)?;
            if request.arguments.len() > 1 {
                self.flow_input_enabled = integer_argument_value(request, 1)? != 0;
            }
            if request.arguments.len() > 2 {
                self.flow_input_can_skip = integer_argument_value(request, 2)? != 0;
            }
            if request.arguments.len() > 3 {
                self.flow_input_force_skip = integer_argument_value(request, 3)? != 0;
            }
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "FLOWINPUTS" {
            *status = HostDispatchStatus::Handled;
            self.flow_input_string = integer_argument_value(request, 0)? != 0;
            if request.arguments.len() > 1 {
                string_argument_value(request, 1, &name)?
                    .clone_into(&mut self.flow_input_default_string);
            }
            return commit_integer_result(vm, request.id, 0);
        }
        if name == "BREAKBUTTON" {
            *status = HostDispatchStatus::Handled;
            self.button_generation = self.button_generation.saturating_add(1);
            self.presentation
                .set_button_generation(self.button_generation);
            self.command_intents.clear();
            commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()))?;
            return self.emit_projection_state();
        }
        Ok(())
    }
}
