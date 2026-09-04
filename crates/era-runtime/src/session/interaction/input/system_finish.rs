impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn finish_system_input(
        &mut self,
        pending: PendingInput,
        value: &VmValue,
    ) -> Result<(), RuntimeError> {
        if self.controller.step != SystemStep::None && self.system_menu_host_request.is_none() {
            return self.finish_flow_input(pending, value);
        }
        match (self.system_menu, value) {
            (SystemMenuState::Title, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                let prepared = vm
                    .prepare_runtime_state(VmRuntimeStateTransaction::ResetNewGame)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                vm.commit_runtime_state(prepared)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                let draw_line = vm
                    .vm()
                    .artifact()
                    .project_data
                    .static_data
                    .replace
                    .draw_line_string
                    .clone();
                self.presentation.append_separator(draw_line);
                self.presentation.append_text(String::new(), false);
                self.controller.flow = Some(SystemFlow::First);
                if !self
                    .controller
                    .prepare_event(vm.vm().artifact(), "EVENTFIRST")
                {
                    return Err(RuntimeError::Internal("EVENTFIRST is not defined".into()));
                }
                let entry = self.controller.next().expect("prepared EVENTFIRST entry");
                let fiber = vm
                    .spawn_entry(entry, Vec::new())
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                self.controller.started(fiber);
                self.set_phase(RuntimePhase::Running)
            }
            (SystemMenuState::Title, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                let vm = self
                    .vm
                    .as_mut()
                    .ok_or_else(|| RuntimeError::Internal("system wait has no VM".into()))?;
                if self
                    .controller
                    .prepare_function(vm.vm().artifact(), "TITLE_LOADGAME")
                {
                    self.controller.flow = Some(SystemFlow::Title);
                    self.controller.step = SystemStep::TitleLoadOverride;
                    let entry = self
                        .controller
                        .next()
                        .expect("prepared TITLE_LOADGAME entry");
                    let fiber = vm
                        .spawn_entry(entry, Vec::new())
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    self.controller.started(fiber);
                    return self.set_phase(RuntimePhase::Running);
                }
                self.issue_storage(
                    PendingStorage::ListLoadSlots,
                    StorageNamespace::Save,
                    StorageOperation::List {
                        pattern: Some("save*.sav".into()),
                        recursive: false,
                    },
                    String::new(),
                )
            }
            (
                SystemMenuState::LoadSlots | SystemMenuState::SaveSlots,
                VmValue::Integer(selection),
            ) if *selection <= -1_000 => {
                let index = usize::try_from(selection.saturating_neg().saturating_sub(1_000))
                    .unwrap_or(usize::MAX);
                let Some(path) = self.load_slot_paths.get(index).cloned() else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown delete slot");
                };
                let save = self.system_menu == SystemMenuState::SaveSlots;
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::StatDeleteMenuSlot {
                        save,
                        path: path.clone(),
                    },
                    StorageNamespace::Save,
                    StorageOperation::Stat,
                    path,
                )
            }
            (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots, VmValue::Integer(100)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.resume_system_menu_host()
            }
            (
                menu @ (SystemMenuState::LoadSlots | SystemMenuState::SaveSlots),
                VmValue::Integer(selection),
            ) if *selection >= 0 && (*selection != 99 || menu == SystemMenuState::SaveSlots) => {
                let slot_count = self
                    .project_snapshot
                    .as_ref()
                    .map_or(20, |snapshot| snapshot.save_slot_count)
                    .max(20);
                let Ok(slot) = u32::try_from(*selection) else {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                };
                if slot >= slot_count {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "unknown save slot");
                }
                let target_page = slot / 20;
                if target_page != self.system_menu_page {
                    self.close_wait(pending.wait.wait_id)?;
                    self.system_menu_page = target_page;
                    return self.scan_slot_page(menu == SystemMenuState::SaveSlots);
                }
                let path = save_slot_path(slot);
                if menu == SystemMenuState::SaveSlots {
                    self.close_wait(pending.wait.wait_id)?;
                    if self.occupied_slot_paths.contains(&path) {
                        self.system_menu = SystemMenuState::ConfirmOverwrite { slot };
                        self.presentation.append_system_text(
                            localized_system_text(
                                &self.selected_locale,
                                SystemTextKey::OverwriteQuestion,
                            ),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(i64::from(slot))],
                            false,
                        );
                        let yes = self.allocate_interaction();
                        let no = self.allocate_interaction();
                        self.presentation.append_system_button(
                            "Yes".into(),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(0)],
                            yes,
                        );
                        self.presentation.append_system_button(
                            "No".into(),
                            SystemTextKey::OverwriteQuestion,
                            vec![SystemTextArgument::Integer(1)],
                            no,
                        );
                        let submission = self.allocate_interaction();
                        let mut wait = self.system_wait(submission);
                        wait.kind = WaitKind::IntegerValue;
                        return self.open_wait(
                            PendingInput {
                                host_request: self.system_menu_host_request,
                                wait,
                                result_name: None,
                                choices: BTreeMap::from([
                                    (yes, VmValue::Integer(0)),
                                    (no, VmValue::Integer(1)),
                                ]),
                                timeout_duration_ns: None,
                                post_input: None,
                            },
                            true,
                        );
                    }
                    return self.begin_system_menu_candidate(slot);
                }
                if !self.occupied_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(0, CommandErrorCode::InvalidValue, "save slot is empty");
                }
                if self.invalid_slot_paths.contains(&path) {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "save slot is incompatible or corrupt",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::LoadSlots, VmValue::Integer(99)) => {
                let path = save_slot_path(99);
                if !self.occupied_slot_paths.contains(&path)
                    || self.invalid_slot_paths.contains(&path)
                {
                    self.operations.restore_active_input(pending);
                    return self.reject(
                        0,
                        CommandErrorCode::InvalidValue,
                        "autosave is unavailable",
                    );
                }
                self.close_wait(pending.wait.wait_id)?;
                self.issue_storage(
                    PendingStorage::ReadLoadSlot { slot: 99 },
                    StorageNamespace::Save,
                    StorageOperation::Read,
                    path,
                )
            }
            (SystemMenuState::ConfirmOverwrite { slot }, VmValue::Integer(0)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.begin_system_menu_candidate(slot)
            }
            (SystemMenuState::ConfirmOverwrite { .. }, VmValue::Integer(1)) => {
                self.close_wait(pending.wait.wait_id)?;
                self.system_menu = SystemMenuState::SaveSlots;
                self.render_slot_menu(true)
            }
            _ => {
                if self.presentation.last_line_is_temporary()
                    && self.presentation.last_line_is_empty()
                {
                    self.presentation.delete_last_lines(2);
                    self.presentation.append_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        true,
                    );
                } else {
                    self.presentation
                        .replace_last_temporary(localized_system_text(
                            &self.selected_locale,
                            SystemTextKey::InvalidValue,
                        ));
                }
                self.operations.restore_active_input(pending);
                self.emit_presentation()?;
                self.reject(
                    0,
                    CommandErrorCode::InvalidValue,
                    "unknown system menu item",
                )
            }
        }
    }
}
