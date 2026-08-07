//! Session construction, protocol dispatch, project loading, and game startup.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    /// Return the current project's selectable traditional-save slot count.
    #[must_use]
    pub fn traditional_save_slot_count(&self) -> Option<u32> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.save_slot_count.max(20))
    }

    /// Fully validate an ordinary save against the active compiled project without mutating it.
    ///
    /// # Errors
    ///
    /// Returns a categorized error when no project is ready, the save is malformed, belongs to a
    /// different game/version, or cannot be restored with the active project schema.
    pub fn inspect_traditional_save(
        &self,
        bytes: &[u8],
    ) -> Result<TraditionalSaveInspection, TraditionalSaveValidationError> {
        let artifact = self
            .artifact
            .as_ref()
            .ok_or(TraditionalSaveValidationError::ProjectUnavailable)?;
        let decoded = decode_era_save(bytes, artifact.artifact())
            .map_err(|error| TraditionalSaveValidationError::Invalid(error.to_string()))?;
        let project_data = &artifact.artifact().project_data;
        let game = &project_data.static_data.game_base;
        if decoded.state.unique_code != game.unique_code {
            return Err(TraditionalSaveValidationError::DifferentGame);
        }
        if !project_data
            .save_load_context()
            .compatibility
            .accepts(decoded.state.unique_code, decoded.state.version)
        {
            return Err(TraditionalSaveValidationError::DifferentVersion);
        }
        let vm = RuntimeVm::new(artifact.clone(), self.options.vm_config);
        vm.prepare_runtime_state_with_extensions(
            VmRuntimeStateTransaction::RestoreOrdinary(Box::new(decoded.state)),
            StructuredScope::Ordinary,
            &decoded.structured_extensions,
        )
        .map_err(|error| TraditionalSaveValidationError::Incompatible(error.to_string()))?;
        Ok(TraditionalSaveInspection {
            description: decoded.description,
        })
    }

    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "the actor constructor explicitly initializes every protocol state field"
    )]
    pub fn new(options: RuntimeOptions) -> Self {
        Self {
            options,
            project_progress_reporter: None,
            state: SessionState::Negotiating,
            phase: RuntimePhase::Negotiating,
            revision: 0,
            epoch: SessionEpoch(0),
            expected_inbound_sequence: 0,
            expected_debug_sequence: 0,
            outbound_sequence: 0,
            debug_outbound_sequence: 0,
            next_message_id: 1,
            next_request_id: 1,
            next_wait_id: 1,
            next_interaction_id: 1,
            next_transfer_id: 1,
            next_effect_id: 1,
            logical_time_ns: 0,
            frontend_time_origin: None,
            random_seed: None,
            negotiated_features: BTreeSet::new(),
            configuration_profile: ConfigurationClientProfile::Reference,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            outbound_journal: BTreeMap::new(),
            effect_journal: BTreeMap::new(),
            accepted_message_ids: BTreeMap::new(),
            accepted_debug_message_ids: BTreeMap::new(),
            active_debug_grant: None,
            next_debug_grant_id: 1,
            debug_resume_phase: None,
            debug_frontend_time_sample: None,
            artifact: None,
            incremental: IncrementalState::default(),
            extension_declarations: Vec::new(),
            vm: None,
            presentation: PresentationModel::default(),
            pending_presentation_update: false,
            operations: PendingOperations::default(),
            key_toggle_state: [0; 256],
            hotkey_state: Vec::new(),
            key_macros: KeyMacros::default(),
            queued_input: VecDeque::new(),
            text_box: String::new(),
            text_box_layout: TextBoxLayout::default(),
            flow_input_enabled: false,
            flow_input_default: 0,
            flow_input_can_skip: false,
            flow_input_force_skip: false,
            flow_input_string: false,
            flow_input_default_string: String::new(),
            button_generation: 0,
            debug_output: String::new(),
            debug_output_base: 0,
            debug_output_subscribed: false,
            projection_environment_revision: 0,
            projection_space_revision: 0,
            client_width: 760,
            client_height: 480,
            line_columns: DEFAULT_LINE_COLUMNS,
            message_skip: false,
            skip_print: false,
            user_defined_skip: false,
            saved_skip: false,
            force_kana_mode: 0,
            client_focused: true,
            client_audio_available: true,
            command_intents: BTreeMap::new(),
            reusable_system_intents: BTreeMap::new(),
            exit_requested: None,
            controller: SystemController::default(),
            undo_checkpoint: None,
            undo_replay: None,
            undo_token: None,
            project_snapshot: None,
            pending_configuration_update: None,
            selected_locale: "ja".into(),
            available_fonts: BTreeSet::new(),
            service_capabilities: BTreeMap::new(),
            storage_capabilities: StorageCapabilities {
                revisions: false,
                atomic_replace: false,
                missing_precondition: false,
                delete: false,
            },
            save_extensions: Vec::new(),
            system_menu: SystemMenuState::Title,
            load_slot_paths: Vec::new(),
            occupied_slot_paths: BTreeSet::new(),
            slot_change_tokens: BTreeMap::new(),
            slot_labels: BTreeMap::new(),
            invalid_slot_paths: BTreeSet::new(),
            system_menu_host_request: None,
            system_menu_page: 0,
            inbound_transfer: None,
            outbound_transfer: None,
            staged_project_manifest: None,
            pending_project_load: None,
            pending_candidate_commit: None,
            candidate_clock: None,
            compiled_project_cache: None,
            compiled_cache_diagnostics: Vec::new(),
            compiled_cache_task: None,
            compiled_cache_failure: None,
        }
    }

    /// Install or clear a side-effect-free project workload progress observer.
    ///
    /// The observer can run from compiler worker threads and must not re-enter this session.
    pub fn set_project_progress_reporter(&mut self, reporter: Option<ProjectProgressReporter>) {
        self.project_progress_reporter = reporter;
    }

    /// Decode and queue one frontend envelope without executing runtime work.
    ///
    /// # Errors
    ///
    /// Rejects malformed, oversized, out-of-sequence, or wrong-session envelopes.
    pub fn submit_envelope(&mut self, bytes: &[u8]) -> Result<(), RuntimeError> {
        let envelope = decode_envelope(bytes, self.options.wire_limits)?;
        if envelope.channel == Channel::Debug && self.state != SessionState::Active {
            return Err(RuntimeError::SessionMismatch);
        }
        if self.state == SessionState::Active
            && (envelope.session != Some(self.options.session_id)
                || envelope.session_epoch != Some(self.epoch))
        {
            return Err(RuntimeError::SessionMismatch);
        }
        let envelope_hash = blake3::hash(bytes);
        let (expected_sequence, accepted_ids) = match envelope.channel {
            Channel::Runtime => (
                &mut self.expected_inbound_sequence,
                &mut self.accepted_message_ids,
            ),
            Channel::Debug => (
                &mut self.expected_debug_sequence,
                &mut self.accepted_debug_message_ids,
            ),
        };
        if envelope.sequence < *expected_sequence {
            if accepted_ids.get(&envelope.message_id) == Some(&(envelope.sequence, envelope_hash)) {
                return Ok(());
            }
            return Err(RuntimeError::InvalidSequence {
                expected: *expected_sequence,
                actual: envelope.sequence,
            });
        }
        if envelope.sequence != *expected_sequence {
            return Err(RuntimeError::InvalidSequence {
                expected: *expected_sequence,
                actual: envelope.sequence,
            });
        }
        if self.inbound.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("inbound journal is full"));
        }
        let message_id = envelope.message_id;
        let message = match envelope.channel {
            Channel::Runtime => InboundMessage::Runtime(RuntimeMessage::from_envelope(&envelope)?),
            Channel::Debug => InboundMessage::Debug(DebugMessage::from_envelope(&envelope)?),
        };
        *expected_sequence = expected_sequence.saturating_add(1);
        accepted_ids.insert(message_id, (envelope.sequence, envelope_hash));
        while accepted_ids.len() > self.options.limits.maximum_journal_entries as usize {
            accepted_ids.pop_first();
        }
        self.inbound.push_back((message_id, message));
        Ok(())
    }

    /// Execute a bounded number of actor transitions and VM instructions.
    ///
    /// # Errors
    ///
    /// Returns an error if a queued transition violates a VM or protocol invariant.
    pub fn drive(
        &mut self,
        budget: RuntimeDriveBudget,
    ) -> Result<RuntimeDriveReport, RuntimeError> {
        self.poll_compiled_cache_task()?;
        let transition_limit = budget.maximum_runtime_transitions.max(1);
        let mut transitions = 0;
        let mut instructions = 0;
        while transitions < transition_limit {
            if let Some((message_id, message)) = self.inbound.pop_front() {
                match message {
                    InboundMessage::Runtime(message) => self.handle_message(message_id, message)?,
                    InboundMessage::Debug(message) => {
                        self.handle_debug_message(message_id, message)?;
                    }
                }
                transitions += 1;
                continue;
            }
            if self.phase == RuntimePhase::WaitingInput && !self.queued_input.is_empty() {
                self.consume_queued_input()?;
                transitions += 1;
                continue;
            }
            if self.phase == RuntimePhase::Running && instructions < budget.maximum_vm_instructions
            {
                let remaining = budget.maximum_vm_instructions - instructions;
                let Some(mut vm) = self.vm.take() else {
                    self.fault(FaultCode::Internal, "running phase has no VM", None)?;
                    transitions += 1;
                    continue;
                };
                synchronize_line_count(&mut self.presentation, &mut vm)?;
                let report = vm.drive(
                    RunBudget {
                        maximum_instructions: remaining
                            .min(self.options.limits.maximum_drive_instructions),
                        maximum_host_calls: self.options.limits.maximum_pending_requests,
                        fiber_quantum: RunBudget::default().fiber_quantum,
                    },
                    VmDriveMode::Normal,
                );
                instructions = instructions.saturating_add(report.instructions);
                let stop = report.stop;
                let made_progress = report.instructions != 0 || !report.events.is_empty();
                for event in report.events {
                    self.handle_vm_event(&mut vm, event)?;
                }
                vm.retire_terminal_fibers();
                if self.operations.active_input().is_some()
                    && !vm.has_runnable_fibers()
                    && self.phase == RuntimePhase::Running
                {
                    self.set_phase(RuntimePhase::WaitingInput)?;
                }
                let has_runnable_fibers = vm.has_runnable_fibers();
                self.vm = Some(vm);
                transitions += 1;
                // A synchronous host call temporarily makes its fiber idle, but handling the
                // returned event immediately makes it runnable again. Keep servicing such calls
                // inside this bounded drive so PRINT-heavy scripts do not require one FFI round
                // trip per display fragment. External waits, input, faults, and debug stops still
                // leave no runnable fiber or change phase and therefore cross the caller boundary.
                if self.phase != RuntimePhase::Running
                    || stop == VmPortStop::DebugStopped
                    || !made_progress
                    || !has_runnable_fibers
                {
                    break;
                }
                continue;
            }
            break;
        }
        self.flush_presentation()?;
        let state = if self.phase == RuntimePhase::Faulted {
            RuntimeDriveState::Faulted
        } else if self.phase == RuntimePhase::Stopped {
            RuntimeDriveState::Stopped
        } else if !self.outbound.is_empty() {
            RuntimeDriveState::OutputReady
        } else if !self.inbound.is_empty()
            || (self.phase == RuntimePhase::Running
                && self.vm.as_ref().is_some_and(RuntimeVm::has_runnable_fibers))
        {
            RuntimeDriveState::MoreWork
        } else {
            RuntimeDriveState::Idle
        };
        Ok(RuntimeDriveReport {
            state,
            vm_instructions: instructions,
            runtime_transitions: transitions,
            queued_envelopes: u32::try_from(self.outbound.len()).unwrap_or(u32::MAX),
        })
    }

    #[must_use]
    pub fn poll_envelope(&mut self) -> Option<Vec<u8>> {
        self.outbound.pop_front()
    }

    #[must_use]
    pub const fn phase(&self) -> RuntimePhase {
        self.phase
    }

    #[must_use]
    pub const fn random_seed(&self) -> Option<u64> {
        self.random_seed
    }

    /// Revision retained for deterministic reload staging without filesystem access.
    #[must_use]
    pub fn project_revision(&self) -> Option<u64> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.manifest.project_revision)
    }

    #[must_use]
    pub fn project_sorts_filenames(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.sort_with_filename)
    }

    #[must_use]
    pub fn project_ignored_new_random(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.use_new_random_ignored)
    }

    #[must_use]
    pub fn project_auto_save(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.auto_save)
    }

    #[must_use]
    pub fn project_save_slot_count(&self) -> Option<u32> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.save_slot_count)
    }

    #[must_use]
    pub fn project_money_label(&self) -> Option<&str> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.money_label.as_str())
    }

    #[must_use]
    pub fn project_money_first(&self) -> Option<bool> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.money_first)
    }

    #[must_use]
    pub fn project_maximum_shop_items(&self) -> Option<u32> {
        self.project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.maximum_shop_items)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn handle_message(
        &mut self,
        message_id: u64,
        message: RuntimeMessage,
    ) -> Result<(), RuntimeError> {
        if self.state == SessionState::Negotiating {
            if is_project_load_message(&message) {
                self.clear_staged_project_manifest();
            }
            return match message {
                RuntimeMessage::ClientHello(hello) => self.hello(message_id, &hello),
                _ => self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "ClientHello must be the first message",
                ),
            };
        }
        if self.phase == RuntimePhase::DebugPaused && debugger_suspends_message(&message) {
            if is_project_load_message(&message) {
                self.clear_staged_project_manifest();
            }
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "state-changing runtime messages are suspended by a debugger stop",
            );
        }
        if self.pending_configuration_update.is_some()
            && matches!(
                &message,
                RuntimeMessage::ProjectManifest(_)
                    | RuntimeMessage::ProjectLoad(_)
                    | RuntimeMessage::ReloadProject(_)
                    | RuntimeMessage::ExtensionRegistrySubmit(_)
            )
        {
            if is_project_load_message(&message) {
                self.clear_staged_project_manifest();
            }
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project mutation is suspended while a configuration update is pending",
            );
        }
        match message {
            RuntimeMessage::ProjectManifest(manifest) => {
                let identity = crate::compiled_cache::project_identity(&manifest);
                self.load_project(
                    message_id,
                    &ProjectLoadRequest {
                        identity,
                        manifest: Some(manifest),
                        compiled_cache_transfer_id: None,
                    },
                )
            }
            RuntimeMessage::ProjectLoad(request) => self.load_project(message_id, &request),
            RuntimeMessage::ReturnToTitle(_) => self.return_to_title(message_id),
            RuntimeMessage::ProjectAnalysisRequest(request) => {
                self.analyze_project(message_id, &request)
            }
            RuntimeMessage::KeyMacroProfileSubmit(profile) => {
                self.submit_key_macro_profile(message_id, &profile)
            }
            RuntimeMessage::KeyMacroCommand(command) => {
                self.apply_key_macro_command(message_id, command)
            }
            RuntimeMessage::ExtensionRegistrySubmit(registry) => {
                self.submit_extension_registry(message_id, registry)
            }
            RuntimeMessage::Start(start) => self.start(message_id, &start),
            RuntimeMessage::Input(input) => self.complete_input(message_id, input),
            RuntimeMessage::AdvanceTime(time) if self.phase == RuntimePhase::DebugPaused => {
                self.debug_frontend_time_sample = Some(time.monotonic_time_ns);
                Ok(())
            }
            RuntimeMessage::AdvanceTime(time) => self.advance_time(message_id, time),
            RuntimeMessage::DeviceStateChanged(state) => {
                if self.phase == RuntimePhase::DebugPaused {
                    self.debug_frontend_time_sample = Some(state.monotonic_time_ns);
                } else {
                    self.observe_frontend_time(state.monotonic_time_ns);
                }
                Ok(())
            }
            RuntimeMessage::ClientStateChanged(state) => {
                self.client_focused = state.focused;
                self.client_audio_available = state.audio_available;
                Ok(())
            }
            RuntimeMessage::ProjectionObservation(observation) => {
                self.observe_projection(message_id, observation)
            }
            RuntimeMessage::InputUndoRequest(request) => {
                self.request_input_undo(message_id, &request)
            }
            RuntimeMessage::EffectAcknowledgement(acknowledgement) => {
                self.acknowledge_effects(message_id, acknowledgement)
            }
            RuntimeMessage::ServiceResponse(response) => {
                self.complete_service(message_id, response)
            }
            RuntimeMessage::StateExportRequest(request) => self.export_state(message_id, request),
            RuntimeMessage::StateImportBegin(request) => {
                self.begin_state_import(message_id, request)
            }
            RuntimeMessage::StateImportChunk(chunk) => self.append_state_import(message_id, &chunk),
            RuntimeMessage::StateImportCommit(commit) => {
                self.commit_state_import(message_id, commit)
            }
            RuntimeMessage::StateExportChunkRequest(request) => {
                self.read_state_export(message_id, request)
            }
            RuntimeMessage::StateTransferCancel(cancel) => {
                self.cancel_state_transfer(message_id, cancel)
            }
            RuntimeMessage::ReloadProject(reload) => self.reload_project(message_id, &reload),
            RuntimeMessage::PrepareConfigurationUpdate(request) => {
                self.prepare_configuration_update(message_id, &request)
            }
            RuntimeMessage::FinalizeConfigurationUpdate(request) => {
                self.finalize_configuration_update(message_id, request)
            }
            RuntimeMessage::ShutdownRequest(_) => {
                self.clear_staged_project_manifest();
                self.shutdown(message_id)
            }
            RuntimeMessage::Acknowledge(ack) => {
                self.outbound_journal
                    .retain(|sequence, _| *sequence > ack.through_sequence);
                Ok(())
            }
            RuntimeMessage::Resynchronize(_) => self.resynchronize(message_id),
            RuntimeMessage::StorageResponse(response) => {
                self.complete_storage(message_id, response)
            }
            RuntimeMessage::ClientHello(_)
            | RuntimeMessage::ServerHello(_)
            | RuntimeMessage::VersionRejected(_)
            | RuntimeMessage::ProjectLoadReport(_)
            | RuntimeMessage::ProjectAnalysisReport(_)
            | RuntimeMessage::ConfigurationUpdatePrepared(_)
            | RuntimeMessage::ConfigurationUpdateCommitted(_)
            | RuntimeMessage::KeyMacroStateChanged(_)
            | RuntimeMessage::StateChanged(_)
            | RuntimeMessage::ExitRequested(_)
            | RuntimeMessage::WaitChanged(_)
            | RuntimeMessage::ProjectionState(_)
            | RuntimeMessage::InputUndoStateChanged(_)
            | RuntimeMessage::PresentationSnapshot(_)
            | RuntimeMessage::PresentationDelta(_)
            | RuntimeMessage::EffectBatch(_)
            | RuntimeMessage::StorageRequest(_)
            | RuntimeMessage::ServiceRequest(_)
            | RuntimeMessage::CancelExternalRequest(_)
            | RuntimeMessage::StateExportReady(_)
            | RuntimeMessage::StateImportAccepted(_)
            | RuntimeMessage::StateImportReady(_)
            | RuntimeMessage::StateExportChunk(_)
            | RuntimeMessage::ShutdownReady(_)
            | RuntimeMessage::Fault(_)
            | RuntimeMessage::CommandRejected(_)
            | RuntimeMessage::RuntimeResynchronized(_)
            | RuntimeMessage::Diagnostic(_)
            | RuntimeMessage::Log(_) => self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "message direction is frontend-incompatible",
            ),
        }
    }
}

fn is_project_load_message(message: &RuntimeMessage) -> bool {
    matches!(
        message,
        RuntimeMessage::ProjectManifest(_) | RuntimeMessage::ProjectLoad(_)
    )
}

mod events;
mod project;
mod startup;
