impl RuntimeSession {
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
        self.audio.reset_all();
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
