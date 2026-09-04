#[allow(clippy::needless_borrow, clippy::single_match_else)]
impl RuntimeSession {
    fn dispatch_control_system_flow(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        self.dispatch_control_continuous_train(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_dotrain(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        self.dispatch_control_begin(vm, request, name, status)?;
        if *status == HostDispatchStatus::Handled {
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_control_continuous_train(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "CALLTRAIN" {
            *status = HostDispatchStatus::Handled;
            let Ok(count) = usize::try_from(integer_argument_value(request, 0)?) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "CALLTRAIN count must be non-negative",
                );
            };
            let Some(capacity) = vm
                .variable_dimensions(request.fiber, "SELECTCOM")
                .and_then(|dimensions| dimensions.first().copied())
                .and_then(|value| usize::try_from(value).ok())
            else {
                // Missing/malformed implicit storage is not a script count error.
                return self.fault(
                    FaultCode::VmFault,
                    "CALLTRAIN count must be smaller than SELECTCOM capacity",
                    Some(request.origin.clone()),
                );
            };
            if count >= capacity {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Bounds,
                    "CALLTRAIN count must be smaller than SELECTCOM capacity",
                );
            }
            self.controller.clear_continuous_train();
            for index in 0..count {
                self.controller
                    .continuous_commands
                    .push_back(read_runtime_integer(
                        vm,
                        "SELECTCOM",
                        &[u64::try_from(index + 1).unwrap_or(u64::MAX)],
                        None,
                    )?);
            }
            self.controller.continuous_train = true;
            self.controller.continuous_total = count;
            return commit_completion(vm, request.id, VmHostCompletion::Ready(HostReady::empty()));
        }
        if name == "STOPCALLTRAIN" {
            *status = HostDispatchStatus::Handled;
            if !self.controller.continuous_train {
                return commit_completion(
                    vm,
                    request.id,
                    VmHostCompletion::Ready(HostReady::empty()),
                );
            }
            // ClearCommands invokes CALLTRAINEND without a return address in the
            // reference engine. The STOPCALLTRAIN caller must therefore be
            // discarded before the system controller resumes its current phase.
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.clear();
            self.controller.clear_continuous_train();
            // STOPCALLTRAIN discards the active COM caller. Resume at the
            // post-command source-check phase explicitly instead of depending
            // on whichever value CALLTRAINEND leaves in RESULT:0.
            self.controller.step = SystemStep::TrainSourceCheck;
            self.skip_print = false;
            if !self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
                return self.continue_system_flow(vm);
            }
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_control_dotrain(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if name == "DOTRAIN" {
            *status = HostDispatchStatus::Handled;
            let command = integer_argument_value(request, 0)?;
            let allowed_step = self.controller.allows_dotrain();
            let train_name_count = vm
                .vm()
                .artifact()
                .project_data
                .static_data
                .name_tables
                .get(&erabasic_data::NameTableKind::Train)
                .map_or(0, |table| table.names.len());
            if command < 0
                || self.controller.flow != Some(SystemFlow::Train)
                || !allowed_step
                || usize::try_from(command).map_or(true, |value| value >= train_name_count)
            {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Operation,
                    "DOTRAIN is not valid in this TRAIN phase or its command is outside TRAINNAME",
                );
            }
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.clear();
            self.controller.clear_continuous_train();
            reset_after_show_user(vm)?;
            self.controller.selected_command = Some(command);
            write_runtime_integer(vm, "SELECTCOM", &[], None, command)?;
            fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
            self.controller.step = SystemStep::TrainEventCom;
            if !self.dispatch_system_event(vm, "EVENTCOM")? {
                return self.continue_system_flow(vm);
            }
            return Ok(());
        }
        Ok(())
    }

    fn dispatch_control_begin(
        &mut self,
        vm: &mut RuntimeVm,
        request: &VmHostRequest,
        name: &str,
        status: &mut HostDispatchStatus,
    ) -> Result<(), RuntimeError> {
        if matches!(name, "BEGIN" | "FORCE_BEGIN") {
            *status = HostDispatchStatus::Handled;
            let Some(VmValue::String(keyword)) = request.argument(0) else {
                return self.fault(
                    FaultCode::VmFault,
                    "BEGIN expects a system keyword",
                    Some(request.origin.clone()),
                );
            };
            let Some(flow) = SystemFlow::parse(keyword) else {
                return complete_script_fault(
                    vm,
                    request,
                    erabasic_vm::ScriptFaultKind::Argument,
                    format!("unknown BEGIN system keyword: {keyword}"),
                );
            };
            // BEGIN resets the console style at instruction execution time, even
            // when the actual system transition is deferred by EVENTSHOP.
            self.presentation.reset_style();
            if flow == SystemFlow::Shop {
                self.controller.shop_called_when_normal =
                    self.controller.flow == Some(SystemFlow::Normal);
            }
            self.controller.deferred_flow = Some(flow);
            if vm.fiber_frame_count(request.fiber).unwrap_or(0) > 1 {
                return commit_completion(vm, request.id, VmHostCompletion::ReturnCurrent(None));
            }
            // The pinned fork treats BEGIN and FORCE_BEGIN as the same forced
            // transition. Returning the issuing root approximates ProcessState's
            // Return(0), while retaining the remaining event handlers.
            vm.cancel_fiber(request.fiber)
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let was_system_handler = self.controller.completed(request.fiber, None);
            if was_system_handler && !self.controller.is_complete() {
                return self.spawn_next_event(vm);
            }
            if self.controller.flow == Some(SystemFlow::Shop)
                && self.controller.step == SystemStep::ShopEvent
            {
                return self.continue_system_flow(vm);
            }
            self.controller.clear();
            if self.controller.continuous_train {
                self.controller.clear_continuous_train();
                self.controller.step = SystemStep::TrainBeginAfterCallTrainEnd;
                self.skip_print = true;
                if self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
                    return Ok(());
                }
                self.skip_print = false;
            }
            let flow = self.controller.deferred_flow.take().unwrap_or(flow);
            self.controller.flow = Some(flow);
            return self.begin_flow(vm, flow);
        }
        Ok(())
    }
}
