#[allow(clippy::needless_borrow)]
impl RuntimeSession {
    fn dispatch_presentation_input_and_layout(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        Self::dispatch_presentation_nf_input_policy(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_input_wait(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_presentation_layout_queries(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_presentation_nf_input_policy(
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(
            name,
            "TINPUTNF" | "TINPUTSNF" | "TONEINPUTNF" | "TONEINPUTSNF"
        ) && !vm
            .vm()
            .artifact()
            .manifest
            .compatibility
            .supports_snake_input()
        {
            *status = HostDispatchStatus::Handled;
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Error(erabasic_vm::ExecutionFailure::classified(
                    erabasic_vm::FaultCategory::Permission,
                    erabasic_vm::VmFaultCode::Host,
                    "NF input is unavailable for this compatibility policy",
                )),
            );
        }
        Ok(())
    }

    fn dispatch_presentation_input_wait(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if let Some(mut pending) = input_wait(
            request,
            self.allocate_wait(),
            self.allocate_interaction(),
            self.logical_time_ns,
        ) {
            *status = HostDispatchStatus::Handled;
            self.prepare_pending_input(&mut pending);
            if self.complete_message_skipped_input(vm, request, name, &pending)? {
                return Ok(());
            }
            let stability = match pending.wait.stability {
                WaitStability::StableInput => HostWaitStability::StableInput,
                WaitStability::Transient => HostWaitStability::Transient,
            };
            commit_completion(
                vm,
                request.id,
                VmHostCompletion::Pending {
                    stability,
                    rebind_payload: encode_canonical(&pending.wait)?,
                },
            )?;
            return self.open_wait(pending, false);
        }
        Ok(())
    }

    fn prepare_pending_input(&mut self, pending: &mut PendingInput) {
        // Bind automatic buttons still buffered on the current logical line before
        // transferring the menu choices into the wait. Otherwise a trailing
        // `PRINTFORM "[58] ..."` is rendered as clickable but its token remains
        // outside the active INPUT/INPUTS validator.
        let count = self.presentation.pending_auto_button_values().len();
        let tokens = (0..count)
            .map(|_| self.allocate_interaction())
            .collect::<Vec<_>>();
        for (token, value) in self.presentation.bind_pending_auto_buttons(&tokens) {
            self.command_intents.insert(token, VmValue::Integer(value));
        }
        if matches!(
            pending.wait.kind,
            WaitKind::EnterKey
                | WaitKind::AnyKey
                | WaitKind::IntegerValue
                | WaitKind::StringValue
                | WaitKind::IntegerButton
                | WaitKind::StringButton
        ) {
            pending.choices = std::mem::take(&mut self.command_intents);
        }
        if pending.wait.stop_message_skip {
            self.message_skip = false;
        }
    }

    fn complete_message_skipped_input(
        &self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        pending: &PendingInput,
    ) -> Result<bool, RuntimeError> {
        let timed_value_input = matches!(
            name,
            "TINPUT"
                | "TONEINPUT"
                | "TINPUTS"
                | "TONEINPUTS"
                | "TINPUTNF"
                | "TONEINPUTNF"
                | "TINPUTSNF"
                | "TONEINPUTSNF"
        );
        let untimed_value_input = matches!(
            name,
            "INPUT"
                | "ONEINPUT"
                | "INPUTS"
                | "ONEINPUTS"
                | "BINPUT"
                | "ONEBINPUT"
                | "BINPUTS"
                | "ONEBINPUTS"
        );
        let (mouse_index, can_skip_index) = if timed_value_input { (4, 5) } else { (1, 2) };
        if !self.message_skip
            || !(timed_value_input || untimed_value_input)
            || !matches!(
                request.argument(can_skip_index),
                Some(VmValue::Integer(value)) if *value != i64::MIN
            )
        {
            return Ok(false);
        }
        let mouse = matches!(
            request.argument(mouse_index),
            Some(VmValue::Integer(value)) if *value != 0
        );
        let target = pending.result_name.as_deref().and_then(|result| {
            if mouse {
                global_place_at(vm, result, 1)
            } else {
                global_place(vm, result)
            }
        });
        let value = pending
            .wait
            .default_value
            .as_ref()
            .map_or(VmValue::Integer(0), protocol_to_vm);
        let writes = target
            .map(|target| vec![HostWrite { target, value }])
            .unwrap_or_default();
        commit_completion(
            vm,
            request.id,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        Ok(true)
    }

    fn dispatch_presentation_layout_queries(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "GETLINESTR" {
            *status = HostDispatchStatus::Handled;
            let Some(VmValue::String(pattern)) = request.argument(0) else {
                return self.fault(
                    FaultCode::VmFault,
                    "GETLINESTR expects a string pattern",
                    Some(request.origin.clone()),
                );
            };
            let value = match erabasic_vm::logical_line_string_with_mode(
                pattern,
                usize::try_from(self.line_columns).unwrap_or(usize::MAX),
                vm.character_width_mode(),
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
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: Some(VmValue::String(value)),
                    writes: Vec::new(),
                }),
            );
        }
        if matches!(
            name,
            "CLIENTWIDTH" | "CLIENTHEIGHT" | "PRINTCPERLINE" | "PRINTCLENGTH"
        ) {
            *status = HostDispatchStatus::Handled;
            let project = self.project_snapshot.as_ref().ok_or_else(|| {
                RuntimeError::Internal("layout query has no loaded project".into())
            })?;
            let value = match name {
                "CLIENTWIDTH" => self.client_width,
                "CLIENTHEIGHT" => self.client_height,
                "PRINTCPERLINE" => project.print_c_per_line,
                _ => project.print_c_length,
            };
            let result = VmValue::Integer(i64::from(value));
            let writes = request
                .argument(0)
                .and_then(vm_place)
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: result.clone(),
                    }]
                })
                .unwrap_or_default();
            return commit_completion(
                vm,
                request.id,
                VmHostCompletion::Ready(HostReady {
                    value: writes.is_empty().then_some(result),
                    writes,
                }),
            );
        }
        Ok(())
    }
}
