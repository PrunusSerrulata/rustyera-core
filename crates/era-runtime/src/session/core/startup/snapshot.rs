impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn start_vm_snapshot(
        &mut self,
        message_id: u64,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let maximum =
            usize::try_from(self.options.limits.maximum_transfer_bytes).unwrap_or(usize::MAX);
        let mut payload = match runtime_snapshot::decode(bytes, maximum) {
            Ok(payload) => payload,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("runtime snapshot is invalid: {error}"),
                );
            }
        };
        let replay_digest = crate::input_replay::digest_hex(bytes);
        if payload.input_controller.pending_sequence.is_some()
            || payload.input_controller.next_admission == 0
            || payload.undo_checkpoint.as_ref().is_some_and(|checkpoint| {
                checkpoint.input_controller.pending_sequence.is_some()
                    || checkpoint.input_controller.next_admission == 0
            })
            || payload.operations.has_device_pump()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "snapshot contains unconsumed sequence or device pump",
            );
        }
        if let Some(checkpoint) = &payload.undo_checkpoint {
            let bytes = checkpoint.inputs.iter().try_fold(0_u64, |total, record| {
                total.checked_add(record.storage_bytes()?)
            });
            if bytes != Some(checkpoint.input_history_bytes)
                || bytes.is_none_or(|bytes| bytes > self.options.limits.maximum_transfer_bytes)
                || checkpoint.inputs.len() > self.options.limits.maximum_journal_entries as usize
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "snapshot input provenance limit differs",
                );
            }
        }
        let replay_snapshot_format = format!("runtime_snapshot_v{}", payload.format_version);
        let replay_snapshot_origin = match payload.origin {
            RuntimeSnapshotOrigin::Normal => "normal",
            RuntimeSnapshotOrigin::Debug => "debug",
            RuntimeSnapshotOrigin::Diagnosis => "diagnosis",
        }
        .to_owned();
        let replay_project_identity = crate::input_replay::identity_hex(&payload.project_identity);
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::VmSnapshot {
            payload_digest: replay_digest,
            snapshot_format: replay_snapshot_format,
            snapshot_origin: replay_snapshot_origin,
            original_project_identity: replay_project_identity,
        })?;
        let artifact = self
            .artifact
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let project = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded project identity is missing".into()))?;
        if payload.compatibility != artifact.artifact().manifest.compatibility {
            return self.reject(
                message_id,
                CommandErrorCode::VersionMismatch,
                &format!(
                    "runtime snapshot profile {} does not match active profile {}",
                    payload.compatibility.profile,
                    artifact.artifact().manifest.compatibility.profile
                ),
            );
        }
        if payload.artifact_id != artifact.artifact().manifest.artifact_id
            || payload.project_identity != project.project_identity
            || payload.resource_count != u64::try_from(project.resources.len()).unwrap_or(u64::MAX)
            || payload.selected_locale != self.selected_locale
            || payload.culture_table_version != CULTURE_TABLE_VERSION
            || !payload.operations.is_snapshot_stable()
        {
            return self.reject(
                message_id,
                CommandErrorCode::VersionMismatch,
                "runtime snapshot does not match the exact project or stable-wait contract",
            );
        }
        if let Err(error) = payload
            .resource_graph
            .validate_project_resources(&project.resource_graph)
        {
            return self.reject(
                message_id,
                CommandErrorCode::VersionMismatch,
                &format!("runtime snapshot resources do not match the loaded project: {error}"),
            );
        }
        let mut system_menu = match payload.system_menu {
            0 => SystemMenuState::Title,
            1 => SystemMenuState::LoadSlots,
            2 => SystemMenuState::SaveSlots,
            3 => SystemMenuState::ConfirmOverwrite {
                slot: payload.system_menu_slot.ok_or_else(|| {
                    RuntimeError::Internal("runtime snapshot overwrite menu lacks its slot".into())
                })?,
            },
            _ => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "runtime snapshot contains an unknown system menu",
                );
            }
        };
        if matches!(
            system_menu,
            SystemMenuState::LoadSlots
                | SystemMenuState::SaveSlots
                | SystemMenuState::ConfirmOverwrite { .. }
        ) && payload.system_menu_host_request.is_none()
            && payload.controller.flow != Some(SystemFlow::Title)
        {
            // Older snapshots could retain the built-in load menu after a save
            // had already entered gameplay. The presentation and stable VM wait
            // are authoritative here; reopening that stale menu discards them.
            system_menu = SystemMenuState::Title;
        }
        let vm_snapshot = match VmSnapshot::decode(&payload.vm_snapshot, maximum) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("VM snapshot is invalid: {error}"),
                );
            }
        };
        if vm_snapshot.compatibility() != &payload.compatibility {
            return self.reject(
                message_id,
                CommandErrorCode::VersionMismatch,
                "embedded VM snapshot compatibility identity does not match runtime snapshot",
            );
        }
        let prepared =
            match RuntimeVm::prepare_restore(artifact.clone(), self.options.vm_config, vm_snapshot)
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    return self.reject(
                        message_id,
                        CommandErrorCode::InvalidValue,
                        &format!("VM snapshot cannot be restored: {error}"),
                    );
                }
            };
        let mut expected_requests = payload.operations.input_host_requests();
        expected_requests.sort();
        let mut rebound_requests = RuntimeVm::restore_waits(&prepared)
            .iter()
            .map(|wait| wait.request)
            .collect::<Vec<_>>();
        rebound_requests.sort();
        if expected_requests != rebound_requests {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "runtime and VM snapshot waits do not correspond",
            );
        }
        let snapshot_digest = *blake3::hash(bytes).as_bytes();
        let mut vm = RuntimeVm::commit_restore(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        if let Err(error) = payload
            .operations
            .html_lines
            .validate_snapshot(&vm, payload.epoch)
        {
            return self.reject(message_id, CommandErrorCode::InvalidValue, &error);
        }
        vm.set_line_columns(self.line_columns);
        vm.set_character_width_mode(configured_character_width_mode(
            self.project_snapshot.as_ref(),
        ));
        let sql_restore_ready = self
            .ready_sql_snapshot_restore
            .as_ref()
            .is_some_and(|ready| ready.digest == snapshot_digest);
        if !payload.sql.connections.is_empty() && !sql_restore_ready {
            if self.sql.snapshot().is_err() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "runtime snapshot replacement cannot cross active SQL state",
                );
            }
            return self.begin_sql_snapshot_restore(
                message_id,
                bytes.to_vec(),
                payload.sql.connections.clone(),
            );
        }
        if payload.sql.connections.is_empty() && self.sql.snapshot().is_err() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "runtime snapshot replacement cannot cross active SQL state",
            );
        }
        let replacement_sql = if payload.sql.connections.is_empty() {
            let mut candidate = self.sql.clone();
            candidate.reset_for_project_boundary();
            candidate
        } else {
            self.ready_sql_snapshot_restore
                .take()
                .filter(|ready| ready.digest == snapshot_digest)
                .ok_or_else(|| {
                    RuntimeError::Internal("validated SQL snapshot candidate is missing".into())
                })?
                .candidate_sql
        };
        let old_sql = std::mem::replace(&mut self.sql, replacement_sql);
        let sql_cleanup = (
            old_sql.provider(),
            old_sql
                .connections()
                .map(|(_, connection)| connection.handle)
                .collect::<Vec<_>>(),
        );

        let new_epoch = self.epoch.0.max(payload.epoch).saturating_add(1);
        let mut operations = payload.operations;
        self.next_wait_id = 1;
        self.next_interaction_id = 1;
        let (tokens, waits) = operations.rebind_stable_inputs(
            new_epoch,
            &mut self.next_wait_id,
            &mut self.next_interaction_id,
        );
        let mut presentation = payload.presentation;
        presentation.rebind_interactions(&tokens, &waits);
        let remap_intents = |values: std::collections::BTreeMap<InteractionToken, VmValue>| {
            values
                .into_iter()
                .filter_map(|(token, value)| tokens.get(&token).copied().map(|new| (new, value)))
                .collect()
        };

        self.retained_title_program = None;
        self.epoch = SessionEpoch(new_epoch);
        self.device_input = crate::device_input::DeviceInput::default();
        self.accepted_message_ids.clear();
        self.vm = Some(vm);
        self.presentation = presentation;
        self.presentation
            .clear_transient_sound_compatibility_state();
        self.audio.recover_bgm(self.presentation.bgm_revision());
        self.presentation
            .set_character_width_mode(configured_character_width_mode(
                self.project_snapshot.as_ref(),
            ));
        self.pending_presentation_update = false;
        self.operations = operations;
        self.project_snapshot
            .as_mut()
            .expect("project identity was checked above")
            .resource_graph = payload.resource_graph;
        self.controller = payload.controller;
        self.logical_time_ns = payload.logical_time_ns;
        self.frontend_time_origin = None;
        self.random_seed = payload.random_seed;
        self.message_skip = payload.message_skip;
        self.skip_print = payload.skip_print;
        self.user_defined_skip = payload.user_defined_skip;
        self.saved_skip = payload.saved_skip;
        self.force_kana_mode = payload.force_kana_mode;
        self.hotkey_state = payload.hotkey_state;
        self.key_macros = payload.key_macros;
        self.input_controller = payload.input_controller;
        self.active_input_source = None;
        self.queued_input.clear();
        self.deferred_input_completion = None;
        self.text_box = payload.text_box;
        self.text_box_layout = payload.text_box_layout;
        self.last_projection_state = None;
        self.flow_input_enabled = payload.flow_input_enabled;
        self.flow_input_default = payload.flow_input_default;
        self.flow_input_can_skip = payload.flow_input_can_skip;
        self.flow_input_force_skip = payload.flow_input_force_skip;
        self.flow_input_string = payload.flow_input_string;
        self.flow_input_default_string = payload.flow_input_default_string;
        self.button_generation = payload.button_generation;
        self.debug_output = payload.debug_output;
        self.debug_output_base = payload.debug_output_base;
        self.command_intents = remap_intents(payload.command_intents);
        self.reusable_system_intents = remap_intents(payload.reusable_system_intents);
        self.save_extensions = payload.save_extensions;
        self.system_menu = system_menu;
        self.load_slot_paths = payload.load_slot_paths;
        self.occupied_slot_paths = payload.occupied_slot_paths;
        self.slot_change_tokens.clear();
        self.slot_labels.clear();
        self.invalid_slot_paths.clear();
        self.system_menu_host_request = payload.system_menu_host_request;
        self.system_menu_page = payload.system_menu_page;
        self.undo_checkpoint = payload.undo_checkpoint;
        self.undo_replay = payload.undo_replay;
        self.undo_token = None;
        let origin_warning = match payload.origin {
            RuntimeSnapshotOrigin::Normal => None,
            RuntimeSnapshotOrigin::Debug => Some((
                "runtime.snapshot_restored_from_debug",
                "restored a VM snapshot captured in debug mode",
            )),
            RuntimeSnapshotOrigin::Diagnosis => Some((
                "runtime.snapshot_restored_from_diagnosis",
                "restored a VM snapshot captured for diagnosis",
            )),
        };
        if let Some((code, message)) = origin_warning {
            self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    context: None,
                    code: code.into(),
                    level: RuntimeLogLevel::Warning,
                    message: message.into(),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                Some(message_id),
            )?;
        }
        if matches!(
            self.system_menu,
            SystemMenuState::LoadSlots | SystemMenuState::SaveSlots
        ) {
            let save = self.system_menu == SystemMenuState::SaveSlots;
            self.operations.clear();
            self.emit_sql_cleanup_for(sql_cleanup.0, &sql_cleanup.1)?;
            return self.issue_storage(
                if save {
                    PendingStorage::ListSaveSlots
                } else {
                    PendingStorage::ListLoadSlots
                },
                StorageNamespace::Save,
                StorageOperation::List {
                    pattern: Some("save*.sav".into()),
                    recursive: false,
                },
                String::new(),
            );
        }
        self.emit_sql_cleanup_for(sql_cleanup.0, &sql_cleanup.1)?;
        self.set_phase(RuntimePhase::WaitingInput)?;
        self.renew_debug_grant()?;
        self.install_input_replay(replay_origin);
        self.emit_presentation()
    }

}
