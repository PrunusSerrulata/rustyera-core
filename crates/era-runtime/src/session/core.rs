//! Session construction, protocol dispatch, project loading, and game startup.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[must_use]
    pub fn new(options: RuntimeOptions) -> Self {
        Self {
            options,
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
            line_columns: 75,
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
            pending_project_load: None,
            pending_candidate_commit: None,
            candidate_clock: None,
            compiled_project_cache: None,
            compiled_cache_diagnostics: Vec::new(),
            compiled_cache_task: None,
            compiled_cache_failure: None,
        }
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
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "state-changing runtime messages are suspended by a debugger stop",
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
            RuntimeMessage::ShutdownRequest(_) => self.shutdown(message_id),
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

    pub(super) fn observe_projection(
        &mut self,
        message_id: u64,
        observation: ProjectionObservation,
    ) -> Result<(), RuntimeError> {
        if observation.environment_revision <= self.projection_environment_revision {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "projection environment revision is not newer",
            );
        }
        if observation.presentation_revision != self.presentation.revision() {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "projection observation does not match the canonical presentation",
            );
        }
        let width = u32::try_from(observation.client_size.width.0).ok();
        let height = u32::try_from(observation.client_size.height.0).ok();
        if width.is_none()
            || width == Some(0)
            || height.is_none()
            || height == Some(0)
            || observation.line_columns == 0
            || !observation.transform.is_valid()
            || observation.projection_space_revision < self.projection_space_revision
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "projection dimensions must be positive",
            );
        }
        self.projection_environment_revision = observation.environment_revision;
        self.projection_space_revision = observation.projection_space_revision;
        self.client_width = width.expect("validated projection width");
        self.client_height = height.expect("validated projection height");
        self.line_columns = observation.line_columns;
        self.text_box = observation.text_box;
        Ok(())
    }

    pub(super) fn emit_projection_state(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::ProjectionState(ProjectionState {
                runtime_revision: self.revision,
                text_box: self.text_box.clone(),
                hotkey_state: self.hotkey_state.clone(),
                button_generation: self.button_generation,
                text_box_layout: self.text_box_layout,
            }),
            None,
        )
    }

    pub(super) fn hello(
        &mut self,
        message_id: u64,
        hello: &ClientHello,
    ) -> Result<(), RuntimeError> {
        let supported = VersionRange::exact(RUNTIME_PROTOCOL_VERSION);
        let Some(selected) = negotiate_version(hello.runtime_versions, supported) else {
            self.emit_log(
                RuntimeLogLevel::Error,
                "runtime protocol negotiation failed: runtime protocol 24.0 is required",
            )?;
            return self.emit(
                RuntimeMessage::VersionRejected(VersionRejected {
                    supported,
                    message: "runtime protocol 24.0 is required".into(),
                }),
                Some(message_id),
            );
        };
        self.state = SessionState::Active;
        self.epoch = SessionEpoch(1);
        let limits = intersect_limits(self.options.limits, hello.requested_limits);
        self.options.limits = limits;
        self.options.wire_limits.maximum_envelope_bytes =
            usize::try_from(limits.maximum_envelope_bytes).unwrap_or(usize::MAX);
        self.options.wire_limits.maximum_payload_bytes =
            usize::try_from(limits.maximum_payload_bytes).unwrap_or(usize::MAX);
        let implemented = [
            RuntimeFeature::TraditionalSave,
            RuntimeFeature::VmSnapshot,
            RuntimeFeature::ProjectReload,
            RuntimeFeature::Storage,
            RuntimeFeature::TimedInput,
            RuntimeFeature::ExternalServices,
            RuntimeFeature::StateResynchronization,
            RuntimeFeature::InputUndo,
            RuntimeFeature::ProjectAnalysis,
            RuntimeFeature::KeyMacros,
        ];
        let features: Vec<_> = implemented
            .into_iter()
            .filter(|feature| hello.features.contains(feature))
            .collect();
        self.negotiated_features = features.iter().copied().collect();
        let selected_capabilities = selected_capabilities(&hello.capabilities);
        self.service_capabilities = selected_capabilities
            .services
            .iter()
            .map(|capability| {
                (
                    (capability.kind, capability.operation.clone()),
                    capability.versions.maximum,
                )
            })
            .collect();
        self.storage_capabilities = selected_capabilities.storage;
        self.available_fonts = selected_capabilities
            .available_fonts
            .iter()
            .map(|name| name.to_lowercase())
            .collect();
        self.selected_locale = select_locale(&hello.preferred_locales).into();
        self.presentation.set_projection(
            selected_capabilities.column_cells,
            selected_capabilities.separators,
            selected_capabilities.html,
            selected_capabilities.graphics,
            selected_capabilities.audio,
        );
        self.emit(
            RuntimeMessage::ServerHello(ServerHello {
                selected_version: selected,
                session: self.options.session_id,
                features,
                limits,
                epoch: self.epoch.0,
                selected_capabilities,
                selected_locale: self.selected_locale.clone(),
            }),
            Some(message_id),
        )?;
        self.emit_log(
            RuntimeLogLevel::Debug,
            format!("runtime handshake complete (epoch {})", self.epoch.0),
        )
    }

    pub(super) fn analyze_project(
        &mut self,
        message_id: u64,
        request: &ProjectAnalysisRequest,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::ProjectAnalysis)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "project analysis was not negotiated",
            );
        }
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::LoadingProject | RuntimePhase::Ready
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project analysis requires an idle runtime",
            );
        }
        let return_phase = self.phase;
        self.set_phase(RuntimePhase::AnalyzingProject)?;
        let report = crate::project::analyze_submitted_project_with_extensions(
            request,
            &self.extension_declarations,
        );
        self.emit(
            RuntimeMessage::ProjectAnalysisReport(report),
            Some(message_id),
        )?;
        self.set_phase(return_phase)
    }

    pub(super) fn submit_key_macro_profile(
        &mut self,
        message_id: u64,
        profile: &KeyMacroProfileSubmit,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::KeyMacros)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "key macros were not negotiated",
            );
        }
        let path = era_runtime_protocol::validate_relative_path(&profile.relative_path)?;
        if !path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("macro.txt"))
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "key macro profile must be named macro.txt",
            );
        }
        match &profile.payload {
            FilePayload::Utf8(text) => self.key_macros.load(text),
            FilePayload::IoError(error) if error.kind == FrontendIoErrorKind::NotFound => {
                self.key_macros = KeyMacros::default();
            }
            FilePayload::IoError(_) | FilePayload::Bytes(_) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "key macro profile must be UTF-8 or not-found",
                );
            }
        }
        self.emit(
            RuntimeMessage::KeyMacroStateChanged(self.key_macros.state()),
            Some(message_id),
        )
    }

    pub(super) fn submit_extension_registry(
        &mut self,
        message_id: u64,
        registry: ExtensionRegistrySubmit,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::ExternalServices)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "Host extensions require external services",
            );
        }
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::LoadingProject | RuntimePhase::Ready
        ) || self.project_snapshot.is_some()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "extensions must be registered before loading a project",
            );
        }
        let mut declarations = registry.declarations;
        declarations.sort_by(|left, right| {
            left.era_name
                .to_ascii_uppercase()
                .cmp(&right.era_name.to_ascii_uppercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        for declaration in &mut declarations {
            declaration.operation.make_ascii_lowercase();
        }
        if declarations.iter().any(|declaration| {
            self.service_capabilities
                .get(&(ServiceKind::Extension, declaration.operation.clone()))
                != Some(&declaration.operation_version)
        }) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "each Host extension must match an exactly negotiated Extension service",
            );
        }
        self.extension_declarations = declarations;
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.extension_registry_accepted".into(),
                level: RuntimeLogLevel::Info,
                message: format!(
                    "accepted {} portable Host extension declarations",
                    self.extension_declarations.len()
                ),
                source: None,
            }),
            Some(message_id),
        )
    }

    pub(super) fn apply_key_macro_command(
        &mut self,
        message_id: u64,
        command: KeyMacroCommand,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::KeyMacros)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "key macros were not negotiated",
            );
        }
        let valid = match command {
            KeyMacroCommand::SelectGroup(group) => self.key_macros.select_group(group),
            KeyMacroCommand::Store { group, slot, text } => {
                self.key_macros.store(group, slot, text)
            }
            KeyMacroCommand::Clear { group, slot } => {
                self.key_macros.store(group, slot, String::new())
            }
        };
        if !valid {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "key macro group or slot is out of range",
            );
        }
        let state = self.key_macros.state();
        self.emit(
            RuntimeMessage::KeyMacroStateChanged(state.clone()),
            Some(message_id),
        )?;
        if self.negotiated_features.contains(&RuntimeFeature::Storage) {
            let resume_phase = self.phase;
            return self.issue_storage(
                PendingStorage::KeyMacroWrite { resume_phase },
                StorageNamespace::Project,
                StorageOperation::Write {
                    data: ProtocolBytes::new(state.serialized.into_bytes()),
                    atomic_replace: self.storage_capabilities.atomic_replace,
                    precondition: StoragePrecondition::Any,
                },
                "macro.txt".into(),
            );
        }
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.key_macro_not_persisted".into(),
                level: RuntimeLogLevel::Info,
                message: "key macro state changed in memory; frontend storage was not negotiated"
                    .into(),
                source: None,
            }),
            Some(message_id),
        )
    }

    pub(super) fn load_project(
        &mut self,
        message_id: u64,
        request: &ProjectLoadRequest,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            self.phase,
            RuntimePhase::Negotiating | RuntimePhase::Ready | RuntimePhase::Faulted
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project loading requires an idle runtime",
            );
        }
        let cache_bytes = match request.compiled_cache_transfer_id {
            Some(transfer_id) => {
                let Some(bytes) = self.consume_state_import(
                    message_id,
                    transfer_id,
                    StateExportKind::CompiledProjectCache,
                )?
                else {
                    return Ok(());
                };
                Some(bytes)
            }
            None => None,
        };
        // Loading a new project invalidates both a completed cache and any detached result
        // still being produced for the previous project identity.
        self.compiled_project_cache = None;
        self.compiled_cache_task = None;
        self.compiled_cache_failure = None;
        self.set_phase(RuntimePhase::LoadingProject)?;
        let mut build = match self.build_project_from_cache(request, cache_bytes.as_deref()) {
            Ok(build) => build,
            Err(report) => {
                self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id))?;
                return self.set_phase(RuntimePhase::Ready);
            }
        };
        build.incremental.compact();
        let exact_cache_hit = build
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_hit");
        self.compiled_project_cache = if exact_cache_hit {
            // The validated imported bytes are already the desired opaque export. Re-encoding
            // the multi-gigabyte logical artifact would erase most of the warm-start win.
            cache_bytes.map(Into::into)
        } else {
            // Cache serialization is intentionally lazy. It is a frontend persistence concern
            // and must not add a multi-second zstd pass to the cold-start critical path.
            None
        };
        self.compiled_cache_diagnostics = build
            .report
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.code.starts_with("runtime.compiled_cache_"))
            .cloned()
            .collect();
        self.incremental = build.incremental;
        self.artifact = build.artifact;
        self.project_snapshot = build.snapshot;
        let metadata = self
            .project_snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !build.report.success || metadata.is_empty() {
            return self.finish_project_load(message_id, build.report);
        }
        self.begin_project_image_metadata(message_id, build.report, metadata)
    }

    fn begin_project_image_metadata(
        &mut self,
        message_id: u64,
        mut report: ProjectLoadReport,
        metadata: Vec<(String, [u8; 32])>,
    ) -> Result<(), RuntimeError> {
        if self
            .service_capabilities
            .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
            != Some(&IMAGE_METADATA_OPERATION_VERSION)
        {
            report.success = false;
            report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.missing_image_metadata_service".into(),
                level: RuntimeLogLevel::Error,
                message: "resource sprites require the negotiated image_metadata service".into(),
                source: None,
            });
            return self.finish_project_load(message_id, report);
        }
        let remaining_metadata = metadata
            .iter()
            .map(|(path, _)| path.to_ascii_lowercase())
            .collect();
        self.pending_project_load = Some(PendingProjectLoad {
            message_id,
            report,
            remaining_metadata,
            queued_metadata: metadata.into(),
            reload: None,
        });
        self.emit_project_image_metadata_requests()
    }

    pub(super) fn emit_project_image_metadata_requests(&mut self) -> Result<(), RuntimeError> {
        let maximum = self.options.limits.maximum_pending_requests as usize;
        if maximum == 0
            && self
                .pending_project_load
                .as_ref()
                .is_some_and(|pending| !pending.queued_metadata.is_empty())
        {
            return Err(RuntimeError::ResourceLimit(
                "too many pending service requests",
            ));
        }
        while self.operations.total_count() < maximum {
            let Some((relative_path, digest)) = self
                .pending_project_load
                .as_mut()
                .and_then(|pending| pending.queued_metadata.pop_front())
            else {
                break;
            };
            let request_id = self.allocate_request()?;
            self.operations.insert_service(
                request_id,
                PendingService::ProjectImageMetadata {
                    relative_path: relative_path.clone(),
                },
            );
            self.emit(
                RuntimeMessage::ServiceRequest(ServiceRequest {
                    request_id,
                    kind: ServiceKind::Image,
                    operation: IMAGE_METADATA_OPERATION.into(),
                    operation_version: IMAGE_METADATA_OPERATION_VERSION,
                    payload: ProtocolBytes::new(encode_canonical(&ImageMetadataRequest {
                        resource_id: relative_path,
                        content_digest: ProtocolBytes::new(digest),
                    })?),
                    deadline_ns: None,
                }),
                None,
            )?;
        }
        Ok(())
    }

    pub(super) fn build_project_from_cache(
        &self,
        request: &ProjectLoadRequest,
        cache_bytes: Option<&[u8]>,
    ) -> Result<ProjectBuild, ProjectLoadReport> {
        let maximum =
            usize::try_from(self.options.limits.maximum_transfer_bytes).unwrap_or(usize::MAX);
        let mut cache_warning = None;
        let cached =
            cache_bytes.and_then(
                |bytes| match crate::compiled_cache::decode(bytes, maximum) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        cache_warning = Some(error);
                        None
                    }
                },
            );
        let expected_key =
            crate::compiled_cache::project_key(&request.identity, &self.extension_declarations);
        let mut build = match cached {
            Some(exact) if exact.key == expected_key => {
                exact_cached_project(exact, request.identity.project_revision)
            }
            cached => {
                let Some(manifest) = request.manifest.as_ref() else {
                    let mut diagnostics = Vec::new();
                    if let Some(error) = cache_warning.take() {
                        diagnostics.push(ProtocolDiagnostic {
                            code: "runtime.compiled_cache_ignored".into(),
                            level: RuntimeLogLevel::Warning,
                            message: error,
                            source: None,
                        });
                    }
                    diagnostics.push(ProtocolDiagnostic {
                        code: "runtime.project_payload_required".into(),
                        level: RuntimeLogLevel::Info,
                        message: "compiled cache is missing or does not match the project".into(),
                        source: None,
                    });
                    return Err(ProjectLoadReport {
                        project_revision: request.identity.project_revision,
                        success: false,
                        diagnostics,
                        payload_required: true,
                    });
                };
                let actual_identity = crate::compiled_cache::project_identity(manifest);
                if actual_identity.source_digest != request.identity.source_digest {
                    return Err(ProjectLoadReport {
                        project_revision: request.identity.project_revision,
                        success: false,
                        diagnostics: vec![ProtocolDiagnostic {
                            code: "runtime.project_identity_mismatch".into(),
                            level: RuntimeLogLevel::Error,
                            message: "submitted project payload differs from its source identity"
                                .into(),
                            source: None,
                        }],
                        payload_required: false,
                    });
                }
                let previous_incremental = cached
                    .as_ref()
                    .map_or(&self.incremental, |value| &value.incremental);
                let previous_artifact = cached
                    .as_ref()
                    .map(|value| value.artifact.artifact())
                    .or_else(|| self.vm.as_ref().map(|vm| vm.vm().artifact()))
                    .or_else(|| self.artifact.as_ref().map(ValidatedArtifact::artifact));
                build_project_with_extensions(
                    manifest,
                    Some(previous_incremental),
                    previous_artifact,
                    &self.extension_declarations,
                )
            }
        };
        if let Some(error) = cache_warning {
            build.report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.compiled_cache_ignored".into(),
                level: RuntimeLogLevel::Warning,
                message: error,
                source: None,
            });
        }
        Ok(build)
    }

    pub(super) fn finish_project_load(
        &mut self,
        message_id: u64,
        report: ProjectLoadReport,
    ) -> Result<(), RuntimeError> {
        if report.success {
            self.undo_checkpoint = None;
            self.undo_replay = None;
            self.undo_token = None;
            if let Some(snapshot) = &self.project_snapshot {
                self.key_macros.set_enabled(matches!(
                    snapshot.configuration.get_code("UseKeyMacro"),
                    Some(erabasic_config::ConfigValue::Boolean(true))
                ));
                self.presentation.configure_project(snapshot);
            }
            let canvas_defaults = (
                self.presentation.default_foreground_rgb(),
                self.presentation.default_background_rgb(),
                self.presentation.font(),
                u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
            );
            if let Some(snapshot) = &mut self.project_snapshot {
                snapshot.resource_graph.configure_canvas_defaults(
                    canvas_defaults.0,
                    canvas_defaults.1,
                    canvas_defaults.2,
                    canvas_defaults.3,
                );
            }
            self.sync_resource_replay();
        } else {
            self.artifact = None;
            self.project_snapshot = None;
        }
        let success = report.success;
        self.emit(RuntimeMessage::ProjectLoadReport(report), Some(message_id))?;
        self.set_phase(if success {
            RuntimePhase::Ready
        } else {
            RuntimePhase::Faulted
        })
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn reload_project(
        &mut self,
        message_id: u64,
        reload: &ReloadProject,
    ) -> Result<(), RuntimeError> {
        let previous_phase = self.phase;
        if !matches!(
            previous_phase,
            RuntimePhase::Ready | RuntimePhase::Running | RuntimePhase::WaitingInput
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project reload requires a ready or running runtime",
            );
        }
        if self.operations.total_count() != 0 && !self.operations.is_snapshot_stable() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "project reload cannot cross transient runtime operations",
            );
        }
        let current = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("project reload has no base manifest".into()))?;
        let manifest = match apply_project_delta(&current.manifest, reload) {
            Ok(manifest) => manifest,
            Err(error) => {
                return self.reject(message_id, CommandErrorCode::InvalidValue, &error);
            }
        };
        self.set_phase(RuntimePhase::Reloading)?;
        let previous_artifact = self
            .vm
            .as_ref()
            .map(|vm| vm.vm().artifact())
            .or_else(|| self.artifact.as_ref().map(ValidatedArtifact::artifact));
        let mut build = build_project_with_extensions(
            &manifest,
            Some(&self.incremental),
            previous_artifact,
            &self.extension_declarations,
        );
        if !build.report.success {
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        if let (Some(next), Some(previous)) =
            (build.snapshot.as_mut(), self.project_snapshot.as_ref())
        {
            next.resource_graph
                .inherit_runtime_graph(&previous.resource_graph);
        }
        let metadata = build
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.resource_graph.metadata_requests())
            .unwrap_or_default();
        if !metadata.is_empty() {
            if self
                .service_capabilities
                .get(&(ServiceKind::Image, IMAGE_METADATA_OPERATION.into()))
                != Some(&IMAGE_METADATA_OPERATION_VERSION)
            {
                build.report.success = false;
                build.report.diagnostics.push(ProtocolDiagnostic {
                    code: "runtime.missing_image_metadata_service".into(),
                    level: RuntimeLogLevel::Error,
                    message:
                        "changed image resources require the negotiated image_metadata service"
                            .into(),
                    source: None,
                });
                self.emit(
                    RuntimeMessage::ProjectLoadReport(build.report),
                    Some(message_id),
                )?;
                return self.set_phase(previous_phase);
            }
            let remaining_metadata = metadata
                .iter()
                .map(|(path, _)| path.to_ascii_lowercase())
                .collect();
            let report = build.report.clone();
            self.pending_project_load = Some(PendingProjectLoad {
                message_id,
                report,
                remaining_metadata,
                queued_metadata: metadata.into(),
                reload: Some(PendingProjectReload {
                    build,
                    previous_phase,
                }),
            });
            return self.emit_project_image_metadata_requests();
        }
        self.commit_project_reload(message_id, build, previous_phase)
    }

    pub(super) fn commit_project_reload(
        &mut self,
        message_id: u64,
        mut build: crate::project::ProjectBuild,
        previous_phase: RuntimePhase,
    ) -> Result<(), RuntimeError> {
        let target = build
            .artifact
            .take()
            .ok_or_else(|| RuntimeError::Internal("successful reload has no artifact".into()))?;
        if let Some(vm) = &mut self.vm
            && let Err(error) = vm.prepare_hot_reload(target.clone())
        {
            build.report.success = false;
            build.report.diagnostics.push(ProtocolDiagnostic {
                code: "runtime.hot_reload_incompatible".into(),
                level: RuntimeLogLevel::Error,
                message: error.to_string(),
                source: None,
            });
            self.emit(
                RuntimeMessage::ProjectLoadReport(build.report),
                Some(message_id),
            )?;
            return self.set_phase(previous_phase);
        }
        if let Some(vm) = &mut self.vm {
            vm.commit_hot_reload()
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        }

        self.artifact = Some(target);
        build.incremental.compact();
        self.incremental = build.incremental;
        self.project_snapshot = build.snapshot;
        self.compiled_cache_diagnostics = build
            .report
            .diagnostics
            .iter()
            .filter(|diagnostic| !diagnostic.code.starts_with("runtime.compiled_cache_"))
            .cloned()
            .collect();
        self.compiled_project_cache = None;
        self.compiled_cache_task = None;
        self.compiled_cache_failure = None;
        if let Some(snapshot) = &self.project_snapshot {
            self.presentation.configure_project(snapshot);
        }
        let canvas_defaults = (
            self.presentation.default_foreground_rgb(),
            self.presentation.default_background_rgb(),
            self.presentation.font(),
            u8::try_from(self.presentation.style_bits()).unwrap_or_default(),
        );
        if let Some(snapshot) = &mut self.project_snapshot {
            snapshot.resource_graph.configure_canvas_defaults(
                canvas_defaults.0,
                canvas_defaults.1,
                canvas_defaults.2,
                canvas_defaults.3,
            );
        }
        self.sync_resource_replay();
        let new_epoch = self.epoch.0.saturating_add(1);
        let (tokens, waits) = self.operations.rebind_stable_inputs(
            new_epoch,
            &mut self.next_wait_id,
            &mut self.next_interaction_id,
        );
        self.presentation.rebind_interactions(&tokens, &waits);
        self.command_intents = std::mem::take(&mut self.command_intents)
            .into_iter()
            .filter_map(|(old, value)| tokens.get(&old).copied().map(|new| (new, value)))
            .collect();
        self.reusable_system_intents = std::mem::take(&mut self.reusable_system_intents)
            .into_iter()
            .filter_map(|(old, value)| tokens.get(&old).copied().map(|new| (new, value)))
            .collect();
        self.epoch = SessionEpoch(new_epoch);
        self.accepted_message_ids.clear();
        self.accepted_debug_message_ids.clear();
        self.invalidate_input_undo(Some(
            "successful bytecode hot reload invalidated the Ctrl-Z checkpoint",
        ))?;
        self.emit(
            RuntimeMessage::ProjectLoadReport(build.report),
            Some(message_id),
        )?;
        self.set_phase(previous_phase)?;
        self.renew_debug_grant()?;
        self.emit_presentation()
    }

    pub(super) fn start(
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

    pub(super) fn start_traditional_save(
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
        let version = decoded.state.version;
        let description = decoded.description.clone();
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
                text: description,
            })
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(last_load)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
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
        self.renew_debug_grant()
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn start_vm_snapshot(
        &mut self,
        message_id: u64,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let maximum =
            usize::try_from(self.options.limits.maximum_transfer_bytes).unwrap_or(usize::MAX);
        let payload = match runtime_snapshot::decode(bytes, maximum) {
            Ok(payload) => payload,
            Err(error) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("runtime snapshot is invalid: {error}"),
                );
            }
        };
        let artifact = self
            .artifact
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded artifact is missing".into()))?;
        let project = self
            .project_snapshot
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("loaded project identity is missing".into()))?;
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
        let vm = RuntimeVm::commit_restore(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;

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

        self.epoch = SessionEpoch(new_epoch);
        self.accepted_message_ids.clear();
        self.vm = Some(vm);
        self.presentation = presentation;
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
        self.queued_input.clear();
        self.text_box = payload.text_box;
        self.text_box_layout = payload.text_box_layout;
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
                    code: code.into(),
                    level: RuntimeLogLevel::Warning,
                    message: message.into(),
                    source: None,
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
        self.set_phase(RuntimePhase::WaitingInput)?;
        self.renew_debug_grant()?;
        self.emit_presentation()
    }

    pub(super) fn start_new_game(&mut self, seed: u64) -> Result<(), RuntimeError> {
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
        let mut vm = RuntimeVm::new_for_title_with_seed(artifact, self.options.vm_config, seed);
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
        self.renew_debug_grant()
    }

    pub(super) fn return_to_title(&mut self, message_id: u64) -> Result<(), RuntimeError> {
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
        self.operations.clear();
        self.effect_journal.clear();
        self.inbound_transfer = None;
        self.outbound_transfer = None;
        self.vm = None;
        self.controller = SystemController::default();
        self.presentation = PresentationModel::default();
        if let Some(project) = &self.project_snapshot {
            self.presentation.configure_project(project);
        }
        self.queued_input.clear();
        self.command_intents.clear();
        self.reusable_system_intents.clear();
        self.undo_checkpoint = None;
        self.undo_replay = None;
        self.undo_token = None;
        self.exit_requested = None;
        self.set_phase(RuntimePhase::Ready)?;
        self.start(
            message_id,
            &StartRequest {
                mode: StartMode::NewGame { seed: None },
            },
        )
    }

    pub(super) fn open_title_menu(&mut self) -> Result<(), RuntimeError> {
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths.clear();
        self.occupied_slot_paths.clear();
        self.slot_change_tokens.clear();
        self.slot_labels.clear();
        self.invalid_slot_paths.clear();
        self.system_menu_host_request = None;
        self.system_menu_page = 0;
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

    pub(super) fn system_wait(&mut self, submission_token: InteractionToken) -> InputWait {
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
        }
    }

    pub(super) fn handle_vm_event(
        &mut self,
        vm: &mut RuntimeVm,
        event: VmPortEvent,
    ) -> Result<(), RuntimeError> {
        match event {
            VmPortEvent::HostCall(request) => self.handle_host_call(vm, &request),
            VmPortEvent::FiberFaulted(_, fault) => self.fault(
                FaultCode::VmFault,
                &fault.message,
                Some(erabasic_vm::VmExecutionOrigin {
                    generation: fault.generation,
                    function: fault.function,
                    function_name: fault.function_name,
                    instruction: fault.instruction,
                    command: fault.command,
                    source: fault.source,
                }),
            ),
            VmPortEvent::FiberCompleted(fiber, value) => {
                if self.controller.completed(fiber, value.as_ref()) {
                    self.spawn_next_event(vm)?;
                    if self.controller.is_complete() && self.controller.deferred_flow.is_some() {
                        if self.controller.flow == Some(SystemFlow::Shop)
                            && self.controller.step == SystemStep::ShopEvent
                        {
                            return self.continue_system_flow(vm);
                        }
                        let flow = self
                            .controller
                            .deferred_flow
                            .take()
                            .expect("checked deferred flow");
                        self.controller.clear();
                        self.controller.flow = Some(flow);
                        return self.begin_flow(vm, flow);
                    }
                    if self.controller.is_complete()
                        && matches!(
                            self.controller.flow,
                            Some(
                                SystemFlow::Title
                                    | SystemFlow::First
                                    | SystemFlow::AfterTrain
                                    | SystemFlow::TurnEnd
                                    | SystemFlow::Normal
                            )
                        )
                    {
                        self.controller.flow = Some(SystemFlow::Normal);
                        return self.fault(
                            FaultCode::VmFault,
                            "script execution ended while the reference system was in NORMAL",
                            None,
                        );
                    }
                    if self.controller.is_complete() && self.controller.step != SystemStep::None {
                        return self.continue_system_flow(vm);
                    }
                }
                Ok(())
            }
            VmPortEvent::FiberYielded(_) => Ok(()),
            VmPortEvent::DebugStopped(stop) => self.enter_debug_stop(stop, None),
        }
    }
}

fn exact_cached_project(
    mut exact: crate::compiled_cache::DecodedCompiledCache,
    project_revision: u64,
) -> ProjectBuild {
    exact.snapshot.manifest.project_revision = project_revision;
    for diagnostic in &mut exact.diagnostics {
        diagnostic.message = format!("[cached] {}", diagnostic.message);
    }
    exact.diagnostics.push(ProtocolDiagnostic {
        code: "runtime.compiled_cache_hit".into(),
        level: RuntimeLogLevel::Debug,
        message: "loaded the exact compiled project cache".into(),
        source: None,
    });
    ProjectBuild {
        artifact: Some(exact.artifact),
        incremental: exact.incremental,
        report: ProjectLoadReport {
            project_revision,
            success: true,
            diagnostics: exact.diagnostics,
            payload_required: false,
        },
        snapshot: Some(exact.snapshot),
    }
}
