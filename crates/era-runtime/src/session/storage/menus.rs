// This is part of the split RuntimeSession storage implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn resume_storage_host(
        &mut self,
        request: erabasic_vm::HostRequestId,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    pub(in super::super) fn open_slot_menu(
        &mut self,
        message_id: u64,
        mut entries: Vec<StorageEntry>,
        save: bool,
    ) -> Result<(), RuntimeError> {
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if entries.iter().any(|entry| {
            era_runtime_protocol::validate_relative_path(&entry.relative_path).is_err()
        }) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "storage list contains an invalid relative path",
            );
        }
        let previous_tokens = std::mem::take(&mut self.slot_change_tokens);
        self.occupied_slot_paths = entries
            .iter()
            .map(|entry| entry.relative_path.clone())
            .collect();
        self.slot_change_tokens = entries
            .into_iter()
            .filter_map(|entry| entry.change_token.map(|token| (entry.relative_path, token)))
            .collect();
        self.slot_labels
            .retain(|path, _| previous_tokens.get(path) == self.slot_change_tokens.get(path));
        self.invalid_slot_paths
            .retain(|path| previous_tokens.get(path) == self.slot_change_tokens.get(path));
        self.system_menu = if save {
            SystemMenuState::SaveSlots
        } else {
            SystemMenuState::LoadSlots
        };
        self.scan_slot_page(save)
    }

    pub(in super::super) fn scan_slot_page(&mut self, save: bool) -> Result<(), RuntimeError> {
        let mut remaining = self.slot_page_paths(save);
        remaining.retain(|path| {
            self.occupied_slot_paths.contains(path) && !self.slot_labels.contains_key(path)
        });
        remaining.reverse();
        self.scan_next_menu_slot(save, remaining)
    }

    pub(in super::super) fn scan_next_menu_slot(
        &mut self,
        save: bool,
        mut remaining: Vec<String>,
    ) -> Result<(), RuntimeError> {
        let Some(path) = remaining.pop() else {
            return self.render_slot_menu(save);
        };
        self.issue_storage(
            PendingStorage::ScanMenuSlot {
                save,
                path: path.clone(),
                remaining,
                data: Vec::new(),
                change_token: self.slot_change_tokens.get(&path).cloned(),
            },
            StorageNamespace::Save,
            StorageOperation::ReadRange {
                offset: 0,
                maximum_bytes: 64 * 1024,
                change_token: self.slot_change_tokens.get(&path).cloned(),
            },
            path,
        )
    }

    fn slot_page_paths(&mut self, save: bool) -> Vec<String> {
        let slot_count = self
            .project_snapshot
            .as_ref()
            .map_or(20, |snapshot| snapshot.save_slot_count)
            .max(20);
        let page_count = slot_count.div_ceil(20);
        self.system_menu_page = self.system_menu_page.min(page_count.saturating_sub(1));
        let start = self.system_menu_page.saturating_mul(20);
        let end = start.saturating_add(20).min(slot_count);
        let mut paths = (start..end).map(save_slot_path).collect::<Vec<_>>();
        if !save {
            paths.push(save_slot_path(99));
        }
        paths
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn render_slot_menu(&mut self, save: bool) -> Result<(), RuntimeError> {
        let slot_count = self
            .project_snapshot
            .as_ref()
            .map_or(20, |snapshot| snapshot.save_slot_count)
            .max(20);
        let page_count = slot_count.div_ceil(20);
        self.system_menu_page = self.system_menu_page.min(page_count.saturating_sub(1));
        self.load_slot_paths = self.slot_page_paths(save);
        let question = if save {
            SystemTextKey::SaveQuestion
        } else {
            SystemTextKey::LoadQuestion
        };
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, question),
            question,
            Vec::new(),
            false,
        );
        let mut choices = BTreeMap::new();
        for index in 0..self.load_slot_paths.len() {
            let path = self.load_slot_paths[index].clone();
            let slot = parse_save_slot(&path).unwrap_or(u32::MAX);
            let occupied = self.occupied_slot_paths.contains(&path);
            let token = self.allocate_interaction();
            let label = if occupied {
                format!(
                    "[{slot:>2}] {}",
                    self.slot_labels
                        .get(&path)
                        .map_or("(unreadable)", String::as_str)
                )
            } else {
                format!("[{slot:>2}] ----")
            };
            self.presentation.append_system_button(
                label,
                SystemTextKey::SaveSlot,
                vec![SystemTextArgument::String(path.clone())],
                token,
            );
            choices.insert(token, VmValue::Integer(i64::from(slot)));
        }
        let back = self.allocate_interaction();
        self.presentation.append_system_button(
            localized_system_text(&self.selected_locale, SystemTextKey::Back),
            SystemTextKey::Back,
            Vec::new(),
            back,
        );
        choices.insert(back, VmValue::Integer(100));
        for page in self.system_menu_page.saturating_add(1)..page_count {
            let first = page.saturating_mul(20);
            let last = first.saturating_add(19).min(slot_count.saturating_sub(1));
            let token = self.allocate_interaction();
            self.presentation.append_system_button(
                format!("[{first}-{last}]"),
                SystemTextKey::SaveSlot,
                vec![SystemTextArgument::Integer(i64::from(first))],
                token,
            );
            choices.insert(token, VmValue::Integer(i64::from(first)));
        }
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        wait.kind = WaitKind::IntegerValue;
        self.open_wait(
            PendingInput {
                host_request: self.system_menu_host_request,
                wait,
                result_name: None,
                choices,
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    pub(in super::super) fn resume_system_menu_host(&mut self) -> Result<(), RuntimeError> {
        let Some(request) = self.system_menu_host_request.take() else {
            return self.open_title_menu();
        };
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths.clear();
        self.occupied_slot_paths.clear();
        self.slot_change_tokens.clear();
        self.slot_labels.clear();
        self.resume_storage_host(request, Vec::new())
    }

    pub(in super::super) fn finish_builtin_autosave(
        &mut self,
        success: bool,
    ) -> Result<(), RuntimeError> {
        if !success {
            return self.stage_builtin_autosave_failure();
        }
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("autosave completion has no VM".into()))?;
        if let Some(flow) = self.controller.deferred_flow.take() {
            self.controller.flow = Some(flow);
            self.begin_flow(&mut vm, flow)?;
            self.vm = Some(vm);
            self.set_phase(RuntimePhase::Running)?;
            return self.renew_debug_grant();
        }
        self.controller.step = SystemStep::ShopShow;
        self.dispatch_system_function(&mut vm, "SHOW_SHOP", true)?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        self.renew_debug_grant()
    }

    pub(in super::super) fn stage_builtin_autosave_failure(&mut self) -> Result<(), RuntimeError> {
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, SystemTextKey::AutoSaveFailed),
            SystemTextKey::AutoSaveFailed,
            Vec::new(),
            false,
        );
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, SystemTextKey::AutoSaveSkipped),
            SystemTextKey::AutoSaveSkipped,
            Vec::new(),
            false,
        );
        self.controller.step = SystemStep::ShopAutosaveFailureWait;
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        wait.kind = WaitKind::EnterKey;
        wait.mouse_input = false;
        wait.default_value = None;
        self.open_wait(
            PendingInput {
                host_request: None,
                wait,
                result_name: None,
                choices: BTreeMap::new(),
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )?;
        self.renew_debug_grant()
    }
}
