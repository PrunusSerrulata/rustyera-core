// This is part of the split RuntimeSession implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn start(
        &mut self,
        message_id: u64,
        request: &StartRequest,
    ) -> Result<(), RuntimeError> {
        let vm_snapshot_restore = matches!(request.mode, StartMode::VmSnapshot { .. });
        if (!vm_snapshot_restore && self.phase != RuntimePhase::Ready)
            || (vm_snapshot_restore
                && !matches!(
                    self.phase,
                    RuntimePhase::Ready | RuntimePhase::WaitingInput | RuntimePhase::Faulted
                ))
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "start requires a loaded project in a replaceable state",
            );
        }
        if let Some(identity) = self
            .project_snapshot
            .as_ref()
            .map(|project| project.manifest.compatibility.clone())
            && identity.is_experimental()
        {
            self.emit(
                RuntimeMessage::Diagnostic(crate::compatibility::experimental_profile_diagnostic(
                    &identity,
                )),
                Some(message_id),
            )?;
        }
        if matches!(request.mode, StartMode::NewGame { .. }) {
            self.advance_epoch();
        }
        match request.mode {
            StartMode::NewGame { seed: Some(seed) } => self.start_new_game(seed),
            StartMode::NewGame { seed: None } => {
                self.set_phase(RuntimePhase::Starting)?;
                let request_id = self.allocate_request()?;
                self.operations
                    .insert_service(request_id, PendingService::StartEntropy);
                self.emit(
                    RuntimeMessage::ServiceRequest(ServiceRequest {
                        request_id,
                        kind: ServiceKind::Entropy,
                        operation: RANDOM_SEED_OPERATION.into(),
                        operation_version: RANDOM_SEED_OPERATION_VERSION,
                        payload: ProtocolBytes::new(encode_canonical(&RandomSeedRequest {})?),
                        deadline_ns: None,
                    }),
                    Some(message_id),
                )
            }
            StartMode::TraditionalSave { transfer_id } => {
                let Some(bytes) = self.consume_state_import(
                    message_id,
                    transfer_id,
                    StateExportKind::TraditionalSave,
                )?
                else {
                    return Ok(());
                };
                self.start_traditional_save(message_id, &bytes)
            }
            StartMode::VmSnapshot { transfer_id } => {
                let Some(bytes) = self.consume_state_import(
                    message_id,
                    transfer_id,
                    StateExportKind::VmSnapshot,
                )?
                else {
                    return Ok(());
                };
                self.start_vm_snapshot(message_id, &bytes)
            }
        }
    }

    pub(in super::super) fn start_traditional_save(
        &mut self,
        message_id: u64,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let artifact = self
            .artifact
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let decoded = match decode_era_save(bytes, artifact.artifact()) {
            Ok(decoded) => decoded,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("traditional save is invalid: {error}"),
                );
            }
        };
        let mut vm = RuntimeVm::new(
            self.artifact
                .clone()
                .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?,
            self.options.vm_config,
        );
        vm.set_line_columns(self.line_columns);
        vm.set_character_width_mode(configured_character_width_mode(
            self.project_snapshot.as_ref(),
        ));
        let version = decoded.state.version;
        let description = decoded.description.clone();
        let replay_digest = crate::input_replay::digest_hex(bytes);
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::TraditionalSave {
            payload_digest: replay_digest,
            description: description.clone(),
            save_version: version.to_string(),
        })?;
        let prepared = match vm.prepare_runtime_state_with_extensions(
            VmRuntimeStateTransaction::RestoreOrdinary(Box::new(decoded.state)),
            StructuredScope::Ordinary,
            &decoded.structured_extensions,
        ) {
            Ok((prepared, _)) => prepared,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("traditional save is incompatible: {error}"),
                );
            }
        };
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let last_load = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::SetLastLoad {
                version,
                slot: -1,
                text: description.clone(),
            })
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(last_load)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.retained_title_program = None;
        self.save_extensions = decoded.opaque_extensions;
        if let Some(project) = &mut self.project_snapshot {
            project.resource_graph.reset_runtime_graph();
        }
        self.sync_resource_replay();
        self.advance_epoch();
        self.system_menu = SystemMenuState::Title;
        self.system_menu_host_request = None;
        self.load_slot_paths.clear();
        self.occupied_slot_paths.clear();
        self.slot_change_tokens.clear();
        self.slot_labels.clear();
        self.invalid_slot_paths.clear();
        self.system_menu_page = 0;
        self.set_phase(RuntimePhase::Starting)?;
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        if self.controller.prepare_load_sequence(vm.vm().artifact()) {
            self.spawn_next_event(&mut vm)?;
        } else {
            self.continue_system_flow(&mut vm)?;
        }
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        self.install_input_replay(replay_origin);
        self.renew_debug_grant()
    }

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

    pub(in super::super) fn start_new_game(&mut self, seed: u64) -> Result<(), RuntimeError> {
        let trigger = self.next_new_game_trigger;
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::NewGame {
            seed: seed.to_string(),
            trigger,
        })?;
        self.random_seed = Some(seed);
        self.frontend_time_origin = None;
        if let Some(project) = &mut self.project_snapshot {
            project.resource_graph.reset_runtime_graph();
        }
        self.sync_resource_replay();
        self.set_phase(RuntimePhase::Starting)?;
        let artifact = self
            .artifact
            .clone()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let title = artifact
            .artifact()
            .project_data
            .static_data
            .game_base
            .title
            .clone();
        self.presentation.set_title(title);
        let reporter = self.project_progress_reporter.clone();
        let mut report_preparation = |progress: erabasic_vm::VmPreparationProgress| {
            let Some(reporter) = &reporter else {
                return;
            };
            reporter.report(ProjectProgress {
                stage: match progress.stage {
                    erabasic_vm::VmPreparationStage::InitializingMemory => {
                        ProjectProgressStage::InitializingMemory
                    }
                    erabasic_vm::VmPreparationStage::IndexingProgram => {
                        ProjectProgressStage::IndexingProgram
                    }
                },
                completed: progress.completed,
                total: progress.total,
            });
        };
        let retained = self
            .retained_title_program
            .take()
            .filter(|retained| retained.artifact_id() == artifact.artifact().manifest.artifact_id);
        let mut vm = if let Some(retained) = retained {
            RuntimeVm::new_for_title_from_retained_program_with_seed_and_progress(
                retained,
                self.options.vm_config,
                seed,
                &mut report_preparation,
            )
        } else {
            RuntimeVm::new_for_title_with_seed_and_progress(
                artifact,
                self.options.vm_config,
                seed,
                &mut report_preparation,
            )
        };
        vm.set_line_columns(self.line_columns);
        vm.set_character_width_mode(configured_character_width_mode(
            self.project_snapshot.as_ref(),
        ));
        self.controller.flow = Some(SystemFlow::Title);
        let result = if self
            .controller
            .prepare_function(vm.vm().artifact(), "SYSTEM_TITLE")
        {
            self.spawn_next_event(&mut vm)?;
            self.vm = Some(vm);
            self.set_phase(RuntimePhase::Running)
        } else {
            self.vm = Some(vm);
            self.open_title_menu()
        };
        result?;
        self.next_new_game_trigger = NewGameTrigger::Start;
        self.install_input_replay(replay_origin);
        self.renew_debug_grant()
    }

    pub(in super::super) fn return_to_title(
        &mut self,
        message_id: u64,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            self.phase,
            RuntimePhase::Ready
                | RuntimePhase::Running
                | RuntimePhase::WaitingInput
                | RuntimePhase::Faulted
        ) || self.project_snapshot.is_none()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "return to title requires a loaded project",
            );
        }
        if self.operations.has_candidate_write() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "return to title cannot cancel an emitted save commit",
            );
        }
        let (services, storage) = self.operations.external_requests();
        self.retained_title_program = self
            .vm
            .take()
            .map(RuntimeVm::retain_program_index_for_title)
            .or_else(|| self.retained_title_program.take());
        self.reset_game_timeline_for_title();
        if let Some(project) = &mut self.project_snapshot {
            project.resource_graph.reset_runtime_graph();
        }
        // Discard the old dirty frame before emitting any protocol message. The generic
        // emitter flushes pending presentation first, which must never serialize the
        // previous game's long history during this memory-retirement transaction.
        self.pending_presentation_update = false;
        self.presentation.reset_preserving_projection();
        if let Some(project) = &self.project_snapshot {
            self.presentation.configure_project(project);
        }
        // Publish the empty authoritative projection before changing phase or requesting
        // entropy. Frontends can release long history, canvases and media while the immutable
        // program index is the only allocation retained from the previous live VM.
        self.emit_presentation()?;
        self.flush_presentation_for_observation()?;
        for request_id in services {
            self.emit(
                RuntimeMessage::CancelExternalRequest(CancelExternalRequest {
                    request_id,
                    kind: ExternalRequestKind::Service,
                }),
                None,
            )?;
        }
        for request_id in storage {
            self.emit(
                RuntimeMessage::CancelExternalRequest(CancelExternalRequest {
                    request_id,
                    kind: ExternalRequestKind::Storage,
                }),
                None,
            )?;
        }
        self.set_phase(RuntimePhase::Ready)?;
        self.next_new_game_trigger = NewGameTrigger::ReturnToTitle;
        self.start(
            message_id,
            &StartRequest {
                mode: StartMode::NewGame { seed: None },
            },
        )
    }

    fn reset_game_timeline_for_title(&mut self) {
        // Retained session/project state: the session-monotonic logical clock, loaded
        // artifact and incremental/static project data, client capabilities and
        // projection parameters, configuration transactions, and compiled cache.
        self.operations = PendingOperations::default();
        self.effect_journal = BTreeMap::new();
        self.inbound_transfer = None;
        self.outbound_transfer = None;
        self.pending_candidate_commit = None;
        self.candidate_clock = None;
        self.controller = SystemController::default();
        self.random_seed = None;
        self.frontend_time_origin = None;
        // `logical_time_ns` is session-monotonic protocol time, not game timeline state.
        // Clearing pending operations removes every old deadline while preserving the
        // non-decreasing clock contract for later frontend samples.
        self.input_replay = InputReplayHistory::default();
        // Envelopes already decoded behind ReturnToTitle still carry the previous epoch.
        // Drop them here so they cannot retain large payloads or act on the new timeline.
        self.inbound = VecDeque::new();
        self.key_toggle_state = [0; 256];
        self.device_input = crate::device_input::DeviceInput::default();
        self.hotkey_state = Vec::new();
        self.queued_input = VecDeque::new();
        self.input_controller = InputController::default();
        self.active_input_source = None;
        self.deferred_input_completion = None;
        self.text_box = String::new();
        self.text_box_layout = TextBoxLayout::default();
        self.flow_input_enabled = false;
        self.flow_input_default = 0;
        self.flow_input_can_skip = false;
        self.flow_input_force_skip = false;
        self.flow_input_string = false;
        self.flow_input_default_string = String::new();
        self.button_generation = 0;
        self.debug_output = String::new();
        self.debug_output_base = 0;
        self.debug_resume_phase = None;
        self.debug_frontend_time_sample = None;
        self.last_projection_state = None;
        self.message_skip = false;
        self.skip_print = false;
        self.user_defined_skip = false;
        self.saved_skip = false;
        self.force_kana_mode = 0;
        self.command_intents = BTreeMap::new();
        self.reusable_system_intents = BTreeMap::new();
        self.exit_requested = None;
        self.undo_checkpoint = None;
        self.undo_replay = None;
        self.undo_token = None;
        self.save_extensions = Vec::new();
        self.reset_title_menu_state();
    }

    fn reset_title_menu_state(&mut self) {
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths = Vec::new();
        self.occupied_slot_paths = BTreeSet::new();
        self.slot_change_tokens = BTreeMap::new();
        self.slot_labels = BTreeMap::new();
        self.invalid_slot_paths = BTreeSet::new();
        self.system_menu_host_request = None;
        self.system_menu_page = 0;
    }

    pub(in super::super) fn open_title_menu(&mut self) -> Result<(), RuntimeError> {
        self.reset_title_menu_state();
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("title menu has no VM".into()))?;
        let static_data = &vm.vm().artifact().project_data.static_data;
        let game_base = static_data.game_base.clone();
        let replace = static_data.replace.clone();
        self.presentation.reset_style();
        self.presentation
            .append_separator(replace.draw_line_string.clone());
        self.presentation.append_text(String::new(), false);
        self.presentation
            .set_alignment(era_runtime_protocol::LineAlignment::Center);
        self.presentation
            .append_text(game_base.title.clone(), false);
        if game_base.version != 0 {
            self.presentation
                .append_text(game_base.script_version_text(), false);
        }
        self.presentation
            .append_text(game_base.author.clone(), false);
        self.presentation
            .append_text(format!("({})", game_base.year), false);
        self.presentation.append_text(String::new(), false);
        self.presentation.append_text(game_base.info.clone(), false);
        self.presentation
            .set_alignment(era_runtime_protocol::LineAlignment::Left);
        self.presentation
            .append_separator(replace.draw_line_string.clone());
        self.presentation.append_text(String::new(), false);
        let start_token = self.allocate_interaction();
        let load_token = self.allocate_interaction();
        let submission_token = self.allocate_interaction();
        self.presentation.append_system_button(
            format!("[0] {}", replace.title_menu_string_0),
            SystemTextKey::NewGame,
            Vec::new(),
            start_token,
        );
        self.presentation.append_system_button(
            format!("[1] {}", replace.title_menu_string_1),
            SystemTextKey::LoadGame,
            Vec::new(),
            load_token,
        );
        let mut wait = self.system_wait(submission_token);
        wait.kind = WaitKind::IntegerValue;
        self.open_wait(
            PendingInput {
                host_request: self.system_menu_host_request,
                wait,
                result_name: None,
                choices: BTreeMap::from([
                    (start_token, VmValue::Integer(0)),
                    (load_token, VmValue::Integer(1)),
                ]),
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    pub(in super::super) fn system_wait(
        &mut self,
        submission_token: InteractionToken,
    ) -> InputWait {
        InputWait {
            wait_id: self.allocate_wait(),
            kind: if self.flow_input_string {
                WaitKind::StringValue
            } else if self.flow_input_enabled {
                WaitKind::IntegerValue
            } else {
                WaitKind::IntegerButton
            },
            stability: WaitStability::StableInput,
            one_input: false,
            stop_message_skip: false,
            system_input: true,
            mouse_input: self.flow_input_enabled,
            default_value: if self.flow_input_string {
                Some(era_runtime_protocol::ProtocolValue::String(
                    self.flow_input_default_string.clone(),
                ))
            } else if self.flow_input_enabled {
                Some(era_runtime_protocol::ProtocolValue::Integer(
                    self.flow_input_default,
                ))
            } else {
                None
            },
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token,
            countdown_remaining_ms: None,
            viewport_policy: era_runtime_protocol::InputViewportPolicy::FollowOutput,
        }
    }
}
