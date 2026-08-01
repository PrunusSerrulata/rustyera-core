// This is part of the split RuntimeSession interaction implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn finish_flow_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        let VmValue::Integer(selection) = value else {
            self.operations.restore_active_input(pending);
            return self.reject(
                0,
                CommandErrorCode::InvalidValue,
                "system input must be integer",
            );
        };
        let previous_choices = pending.choices.clone();
        self.close_wait(pending.wait.wait_id)?;
        self.set_phase(RuntimePhase::Running)?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("system flow input has no VM".into()))?;
        let result = match self.controller.step {
            SystemStep::TrainShowUser => {
                if let Some(command) = usize::try_from(*selection)
                    .ok()
                    .and_then(|index| self.controller.train_commands.get(index))
                    .copied()
                {
                    self.controller.selected_command = Some(command);
                    write_runtime_integer(&mut vm, "SELECTCOM", &[], None, command)?;
                    fill_runtime_variable(&mut vm, "NOWEX", VmValue::Integer(0), true)?;
                    self.controller.step = SystemStep::TrainEventCom;
                    if self.dispatch_system_event(&mut vm, "EVENTCOM")? {
                        Ok(())
                    } else {
                        self.continue_system_flow(&mut vm)
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.controller.step = SystemStep::TrainUserCom;
                    self.dispatch_system_function(&mut vm, "USERCOM", true)?;
                    Ok(())
                }
            }
            SystemStep::AblupShowSelect => {
                self.controller.step = SystemStep::AblupAction;
                if (0..100).contains(selection) {
                    if self.dispatch_system_function(
                        &mut vm,
                        &format!("ABLUP{selection}"),
                        false,
                    )? {
                        Ok(())
                    } else {
                        self.presentation
                            .replace_last_temporary(localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::InvalidValue,
                            ));
                        self.command_intents = previous_choices.clone();
                        self.open_system_command_wait()
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.dispatch_system_function(&mut vm, "USERABLUP", true)?;
                    Ok(())
                }
            }
            SystemStep::ShopShow => {
                let maximum = self
                    .project_snapshot
                    .as_ref()
                    .map_or(100, |snapshot| snapshot.maximum_shop_items);
                if *selection >= 0 && *selection < i64::from(maximum) {
                    let purchase = purchase_item(
                        &mut vm,
                        usize::try_from(*selection).unwrap_or(usize::MAX),
                        maximum,
                    )?;
                    match purchase {
                        PurchaseResult::Purchased => {
                            self.controller.step = SystemStep::ShopAction;
                            if !self.dispatch_system_event(&mut vm, "EVENTBUY")? {
                                self.continue_system_flow(&mut vm)?;
                            }
                            Ok(())
                        }
                        PurchaseResult::OutOfStock | PurchaseResult::NotEnoughMoney => {
                            let key = if purchase == PurchaseResult::NotEnoughMoney {
                                SystemTextKey::NotEnoughMoney
                            } else {
                                SystemTextKey::OutOfStock
                            };
                            self.presentation
                                .replace_last_temporary(localized_system_text(
                                    &self.selected_locale,
                                    key,
                                ));
                            self.command_intents = previous_choices.clone();
                            self.open_system_command_wait()
                        }
                    }
                } else {
                    write_runtime_integer(&mut vm, "RESULT", &[], None, *selection)?;
                    self.controller.step = SystemStep::ShopAction;
                    self.dispatch_system_function(&mut vm, "USERSHOP", true)?;
                    Ok(())
                }
            }
            SystemStep::TrainEventComEndWait => {
                self.controller.step = SystemStep::TrainShowStatus;
                self.dispatch_system_function(&mut vm, "SHOW_STATUS", true)?;
                Ok(())
            }
            SystemStep::ShopAutosaveFailureWait => {
                if let Some(flow) = self.controller.deferred_flow.take() {
                    self.controller.flow = Some(flow);
                    return self.begin_flow(&mut vm, flow).map(|()| {
                        self.vm = Some(vm);
                    });
                }
                self.controller.step = SystemStep::ShopShow;
                self.dispatch_system_function(&mut vm, "SHOW_SHOP", true)?;
                Ok(())
            }
            _ => Err(RuntimeError::Internal(
                "system flow received input outside an input step".into(),
            )),
        };
        self.vm = Some(vm);
        result
    }

    pub(in super::super) fn spawn_next_event(
        &mut self,
        vm: &mut RuntimeVm,
    ) -> Result<(), RuntimeError> {
        if let Some(entry) = self.controller.next() {
            let fiber = vm
                .spawn_entry(entry, Vec::new())
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            self.controller.started(fiber);
        }
        Ok(())
    }

    pub(in super::super) fn begin_flow(
        &mut self,
        vm: &mut RuntimeVm,
        flow: SystemFlow,
    ) -> Result<(), RuntimeError> {
        self.skip_print = false;
        if flow == SystemFlow::Train {
            reset_training_state(vm)?;
            self.controller.train_scan = 0;
            self.controller.train_commands.clear();
            self.controller.clear_continuous_train();
        }
        self.controller.step = match flow {
            SystemFlow::Train => SystemStep::TrainEvent,
            SystemFlow::Ablup => SystemStep::AblupShowJuel,
            SystemFlow::Shop => SystemStep::ShopEvent,
            _ => SystemStep::None,
        };
        let (entry, event, required) = match flow {
            SystemFlow::Title => ("SYSTEM_TITLE", false, false),
            SystemFlow::First => ("EVENTFIRST", true, true),
            SystemFlow::Train => ("EVENTTRAIN", true, false),
            SystemFlow::AfterTrain => ("EVENTEND", true, true),
            SystemFlow::Ablup => ("SHOW_JUEL", false, true),
            SystemFlow::TurnEnd => ("EVENTTURNEND", true, true),
            SystemFlow::Shop => ("EVENTSHOP", true, false),
            SystemFlow::Normal => {
                return self.fault(
                    FaultCode::VmFault,
                    "NORMAL is an internal system state and is not a BEGIN target",
                    None,
                );
            }
        };
        if event {
            if self.controller.prepare_event(vm.vm().artifact(), entry) {
                return self.spawn_next_event(vm);
            }
        } else if self.controller.prepare_function(vm.vm().artifact(), entry) {
            return self.spawn_next_event(vm);
        }
        if required {
            self.fault(
                FaultCode::VmFault,
                &format!("required system function {entry} is not defined"),
                None,
            )
        } else if self.controller.step != SystemStep::None {
            self.continue_system_flow(vm)
        } else {
            Ok(())
        }
    }

    pub(in super::super) fn dispatch_system_function(
        &mut self,
        vm: &mut RuntimeVm,
        name: &str,
        required: bool,
    ) -> Result<bool, RuntimeError> {
        if self.controller.prepare_function(vm.vm().artifact(), name) {
            self.spawn_next_event(vm)?;
            return Ok(true);
        }
        if required {
            self.fault(
                FaultCode::VmFault,
                &format!("required system function {name} is not defined"),
                None,
            )?;
        }
        Ok(false)
    }

    pub(in super::super) fn dispatch_system_event(
        &mut self,
        vm: &mut RuntimeVm,
        name: &str,
    ) -> Result<bool, RuntimeError> {
        if self.controller.prepare_event(vm.vm().artifact(), name) {
            self.spawn_next_event(vm)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(in super::super) fn open_system_command_wait(&mut self) -> Result<(), RuntimeError> {
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        if !self.flow_input_string {
            wait.kind = WaitKind::IntegerValue;
        }
        let choices = std::mem::take(&mut self.command_intents);
        self.reusable_system_intents.clone_from(&choices);
        self.open_wait(
            PendingInput {
                host_request: None,
                wait,
                result_name: None,
                choices,
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn continue_system_flow(
        &mut self,
        vm: &mut RuntimeVm,
    ) -> Result<(), RuntimeError> {
        match self.controller.step {
            SystemStep::TrainEvent => {
                let next = read_runtime_integer(vm, "NEXTCOM", &[], None)?;
                if next >= 0 {
                    write_runtime_integer(vm, "SELECTCOM", &[], None, next)?;
                    write_runtime_integer(vm, "NEXTCOM", &[], None, 0)?;
                    self.controller.selected_command = Some(next);
                    fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
                    self.controller.step = SystemStep::TrainEventCom;
                    if !self.dispatch_system_event(vm, "EVENTCOM")? {
                        return self.continue_system_flow(vm);
                    }
                } else {
                    if self.controller.continuous_train {
                        // Emuera suppresses SHOW_STATUS and the command table while it
                        // rebuilds COM_ABLE for a continuous command.
                        self.skip_print = true;
                    }
                    self.controller.step = SystemStep::TrainShowStatus;
                    self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                }
            }
            SystemStep::TrainShowStatus => {
                self.controller.step = SystemStep::TrainComAble;
                self.controller.train_scan = 0;
                self.controller.train_commands.clear();
                return self.prepare_next_comable(vm);
            }
            SystemStep::TrainComAble => {
                let command = self.controller.train_scan.saturating_sub(1);
                if read_runtime_integer(vm, "RESULT", &[], None)? != 0 {
                    self.controller
                        .train_commands
                        .push(i64::try_from(command).unwrap_or(i64::MAX));
                }
                return self.prepare_next_comable(vm);
            }
            SystemStep::TrainShowUser if self.controller.continuous_train => {
                reset_after_show_user(vm)?;
                self.skip_print = false;
                if let Some(command) = self.controller.continuous_commands.pop_front() {
                    self.controller.continuous_executed =
                        self.controller.continuous_executed.saturating_add(1);
                    let current =
                        i64::try_from(self.controller.continuous_executed).unwrap_or(i64::MAX);
                    let total = i64::try_from(self.controller.continuous_total).unwrap_or(i64::MAX);
                    let text = localized_system_text(
                        &self.selected_locale,
                        SystemTextKey::ContinuousTrainProgress,
                    )
                    .replace("{0}", &current.to_string())
                    .replace("{1}", &total.to_string());
                    self.presentation.append_system_text(
                        text,
                        SystemTextKey::ContinuousTrainProgress,
                        vec![
                            SystemTextArgument::Integer(current),
                            SystemTextArgument::Integer(total),
                        ],
                        false,
                    );
                    if self.controller.train_commands.contains(&command) {
                        self.controller.selected_command = Some(command);
                        write_runtime_integer(vm, "SELECTCOM", &[], None, command)?;
                        fill_runtime_variable(vm, "NOWEX", VmValue::Integer(0), true)?;
                        self.controller.step = SystemStep::TrainEventCom;
                        if !self.dispatch_system_event(vm, "EVENTCOM")? {
                            return self.continue_system_flow(vm);
                        }
                    } else {
                        self.presentation.append_system_text(
                            localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::ContinuousTrainCommandFailed,
                            ),
                            SystemTextKey::ContinuousTrainCommandFailed,
                            Vec::new(),
                            false,
                        );
                        write_runtime_integer(vm, "RESULT", &[], None, command)?;
                        self.controller.step = SystemStep::TrainUserCom;
                        self.dispatch_system_function(vm, "USERCOM", true)?;
                    }
                } else {
                    return self.finish_continuous_train(vm);
                }
            }
            SystemStep::TrainShowUser => {
                reset_after_show_user(vm)?;
                return self.open_system_command_wait();
            }
            SystemStep::AblupShowSelect | SystemStep::ShopShow => {
                return self.open_system_command_wait();
            }
            SystemStep::TrainUserCom => {
                if self.controller.continuous_train
                    && self.controller.continuous_commands.is_empty()
                {
                    return self.finish_continuous_train(vm);
                }
                self.controller.step = SystemStep::TrainShowStatus;
                self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
            }
            SystemStep::TrainEventComEnd => {
                if self.controller.continuous_train
                    && self.controller.continuous_commands.is_empty()
                {
                    return self.finish_continuous_train(vm);
                }
                return self.finish_event_com_end(vm);
            }
            SystemStep::TrainEventCom => {
                let command = self.controller.selected_command.ok_or_else(|| {
                    RuntimeError::Internal("training command selection disappeared".into())
                })?;
                self.controller.step = SystemStep::TrainCommand;
                self.dispatch_system_function(vm, &format!("COM{command}"), true)?;
            }
            SystemStep::TrainCommand => {
                let result = read_runtime_integer(vm, "RESULT", &[], None)?;
                if result == 0 {
                    self.controller.step = SystemStep::TrainShowStatus;
                    self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
                } else {
                    self.controller.step = SystemStep::TrainSourceCheck;
                    self.dispatch_system_function(vm, "SOURCE_CHECK", true)?;
                }
            }
            SystemStep::TrainSourceCheck => {
                fill_runtime_variable(vm, "SOURCE", VmValue::Integer(0), true)?;
                self.controller.step = SystemStep::TrainEventComEnd;
                self.controller.event_com_end_wait_required = true;
                if !self.dispatch_system_event(vm, "EVENTCOMEND")? {
                    return self.continue_system_flow(vm);
                }
            }
            SystemStep::AblupShowJuel => {
                self.controller.step = SystemStep::AblupShowSelect;
                self.dispatch_system_function(vm, "SHOW_ABLUP_SELECT", true)?;
            }
            SystemStep::AblupAction => {
                if self.presentation.last_line_is_temporary() {
                    self.command_intents
                        .clone_from(&self.reusable_system_intents);
                    return self.open_system_command_wait();
                }
                self.controller.step = SystemStep::AblupShowJuel;
                self.dispatch_system_function(vm, "SHOW_JUEL", true)?;
            }
            SystemStep::ShopEvent => {
                if self
                    .project_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.auto_save)
                    && self.controller.shop_called_when_normal
                {
                    self.controller.step = SystemStep::ShopAutosave;
                    if !self.dispatch_system_function(vm, "SYSTEM_AUTOSAVE", false)? {
                        return self.begin_candidate_save(
                            vm,
                            99,
                            CandidateSaveContinuation::Autosave,
                        );
                    }
                } else {
                    if let Some(flow) = self.controller.deferred_flow.take() {
                        self.controller.flow = Some(flow);
                        return self.begin_flow(vm, flow);
                    }
                    self.controller.step = SystemStep::ShopShow;
                    self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
                }
            }
            SystemStep::ShopAutosave | SystemStep::ShopAction | SystemStep::PostLoadShop => {
                if let Some(flow) = self.controller.deferred_flow.take() {
                    self.controller.flow = Some(flow);
                    return self.begin_flow(vm, flow);
                }
                self.controller.step = SystemStep::ShopShow;
                self.dispatch_system_function(vm, "SHOW_SHOP", true)?;
            }
            SystemStep::TitleLoadOverride => {
                self.controller.step = SystemStep::None;
                return self.open_title_menu();
            }
            SystemStep::TrainCallTrainEnd => return self.finish_event_com_end(vm),
            SystemStep::TrainBeginAfterCallTrainEnd => {
                self.skip_print = false;
                let flow = self.controller.deferred_flow.take().ok_or_else(|| {
                    RuntimeError::Internal("deferred BEGIN target disappeared".into())
                })?;
                self.controller.flow = Some(flow);
                return self.begin_flow(vm, flow);
            }
            SystemStep::TrainEventComEndWait
            | SystemStep::ShopAutosaveFailureWait
            | SystemStep::None => {}
        }
        Ok(())
    }

    pub(in super::super) fn prepare_next_comable(
        &mut self,
        vm: &mut RuntimeVm,
    ) -> Result<(), RuntimeError> {
        let names = vm
            .vm()
            .artifact()
            .project_data
            .static_data
            .name_tables
            .get(&erabasic_data::NameTableKind::Train)
            .map(|table| table.names.clone())
            .unwrap_or_default();
        let default_enabled = vm
            .vm()
            .artifact()
            .project_data
            .static_data
            .replace
            .com_able_default
            != 0;
        while self.controller.train_scan < names.len() {
            let command = self.controller.train_scan;
            self.controller.train_scan += 1;
            if names[command].is_none() {
                continue;
            }
            if self.dispatch_system_function(vm, &format!("COM_ABLE{command}"), false)? {
                return Ok(());
            }
            if default_enabled {
                self.controller
                    .train_commands
                    .push(i64::try_from(command).unwrap_or(i64::MAX));
            }
        }
        if !self.controller.continuous_train {
            for (display, command) in self
                .controller
                .train_commands
                .clone()
                .into_iter()
                .enumerate()
            {
                let name = usize::try_from(command)
                    .ok()
                    .and_then(|index| names.get(index))
                    .and_then(Option::as_deref)
                    .unwrap_or("");
                let token = self.allocate_interaction();
                let display = i64::try_from(display).unwrap_or(i64::MAX);
                self.presentation.append_button(
                    format!("{name}[{display:>3}]"),
                    era_runtime_protocol::ProtocolValue::Integer(display),
                    token,
                    None,
                );
                self.command_intents
                    .insert(token, VmValue::Integer(display));
            }
        }
        // The reference system flushes the partial SHOW_STATUS/COM_ABLE row before
        // entering SHOW_USERCOM. In eraTW this separates the final PALAM entry from
        // the following Look section without requiring a script-specific newline.
        self.presentation.flush_pending_line();
        self.controller.step = SystemStep::TrainShowUser;
        self.dispatch_system_function(vm, "SHOW_USERCOM", true)?;
        Ok(())
    }

    fn finish_continuous_train(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        self.controller.clear_continuous_train();
        self.skip_print = false;
        self.controller.step = SystemStep::TrainCallTrainEnd;
        if !self.dispatch_system_function(vm, "CALLTRAINEND", false)? {
            return self.finish_event_com_end(vm);
        }
        Ok(())
    }

    fn finish_event_com_end(&mut self, vm: &mut RuntimeVm) -> Result<(), RuntimeError> {
        if self.controller.event_com_end_wait_required {
            self.controller.event_com_end_wait_required = false;
            self.controller.step = SystemStep::TrainEventComEndWait;
            let submission = self.allocate_interaction();
            let mut wait = self.system_wait(submission);
            wait.kind = WaitKind::EnterKey;
            wait.mouse_input = false;
            wait.default_value = None;
            return self.open_wait(
                PendingInput {
                    host_request: None,
                    wait,
                    result_name: None,
                    choices: BTreeMap::new(),
                    timeout_duration_ns: None,
                    post_input: None,
                },
                true,
            );
        }
        self.controller.step = SystemStep::TrainShowStatus;
        self.dispatch_system_function(vm, "SHOW_STATUS", true)?;
        Ok(())
    }

    pub(in super::super) fn open_wait(
        &mut self,
        pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if self.operations.active_input().is_some() {
            self.operations.queue_input(pending);
            return Ok(());
        }
        self.activate_wait(pending, pause_runtime)
    }

    pub(in super::super) fn activate_wait(
        &mut self,
        mut pending: PendingInput,
        pause_runtime: bool,
    ) -> Result<(), RuntimeError> {
        if self.restart_queued_input_undo()? {
            return Ok(());
        }
        if let Some(duration) = pending.timeout_duration_ns {
            pending.wait.deadline_ns = Some(self.logical_time_ns.saturating_add(duration));
            if pending.wait.display_time {
                pending.wait.countdown_remaining_ms = Some(duration / 1_000_000);
            }
        }
        if let Some(submission) = self.replay_submission(&pending.wait) {
            self.operations.activate_input(pending);
            return self.finish_input(submission, false);
        }
        if self.undo_replay.is_none() {
            self.undo_token = None;
            self.emit_input_undo_state()?;
        }
        let automatic_system_value = (pending.wait.system_input
            && (self.flow_input_force_skip || (self.flow_input_can_skip && self.message_skip)))
            .then(|| {
                if self.flow_input_string {
                    VmValue::String(self.flow_input_default_string.clone())
                } else {
                    VmValue::Integer(self.flow_input_default)
                }
            });
        let count = self.presentation.pending_auto_button_values().len();
        let tokens = (0..count)
            .map(|_| self.allocate_interaction())
            .collect::<Vec<_>>();
        for (token, value) in self.presentation.bind_pending_auto_buttons(&tokens) {
            self.command_intents.insert(token, VmValue::Integer(value));
        }
        // Emuera flushes its print buffer whenever script execution yields for input.
        // Keeping that boundary canonical prevents output after a resumed INPUT from
        // being appended to the menu line that opened the wait.
        self.presentation.flush_pending_line();
        self.presentation.set_wait(Some(pending.wait.clone()));
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Opened(pending.wait.clone())),
            None,
        )?;
        self.operations.activate_input(pending);
        self.emit_presentation()?;
        if let Some(value) = automatic_system_value {
            return self.finish_input(InputSubmission::Value(value), false);
        }
        if pause_runtime {
            self.set_phase(RuntimePhase::WaitingInput)
        } else {
            Ok(())
        }
    }

    pub(in super::super) fn close_wait(&mut self, wait_id: u64) -> Result<(), RuntimeError> {
        self.presentation.set_wait(None);
        self.emit(
            RuntimeMessage::WaitChanged(WaitChange::Closed(wait_id)),
            None,
        )?;
        self.emit_presentation()
    }
}
