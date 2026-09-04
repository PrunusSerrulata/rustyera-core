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
        if self.pending_sql_snapshot_restore.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "start cannot replace a pending exact SQL restore",
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
        let DecodedEraSave {
            state,
            description,
            opaque_extensions,
            structured_extensions,
        } = decoded;
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
        let version = state.version;
        let replay_digest = crate::input_replay::digest_hex(bytes);
        let replay_origin = self.prepare_input_replay(ReplayOriginDetails::TraditionalSave {
            payload_digest: replay_digest,
            description: description.clone(),
            save_version: version.to_string(),
        })?;
        let prepared = match vm.prepare_runtime_state_with_extensions(
            VmRuntimeStateTransaction::RestoreOrdinary(Box::new(state)),
            StructuredScope::Ordinary,
            &structured_extensions,
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
        let load = PreparedTraditionalStart {
            vm,
            opaque_extensions,
            replay_origin,
        };
        self.complete_traditional_start(load)
    }

    fn complete_traditional_start(
        &mut self,
        load: PreparedTraditionalStart,
    ) -> Result<(), RuntimeError> {
        let PreparedTraditionalStart {
            mut vm,
            opaque_extensions,
            replay_origin,
        } = load;
        self.retained_title_program = None;
        self.save_extensions = opaque_extensions;
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
        self.renew_debug_grant()?;
        self.emit_snake_save_load_diagnostic(SaveLoadScope::Ordinary);
        Ok(())
    }

}
