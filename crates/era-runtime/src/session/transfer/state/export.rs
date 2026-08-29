#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super::super) fn export_state(
        &mut self,
        message_id: u64,
        request: StateExportRequest,
    ) -> Result<(), RuntimeError> {
        if request.kind == StateExportKind::FullProjectManifest {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "full project manifest is import-only and cannot be exported",
            );
        }
        if request.kind != StateExportKind::VmSnapshot
            && request.snapshot_purpose != SnapshotExportPurpose::Normal
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "snapshot purpose is only valid for VM snapshot exports",
            );
        }
        if request.kind == StateExportKind::VmSnapshot
            && (!self.queued_input.is_empty()
                || self.input_controller.pending_sequence.is_some()
                || self.undo_checkpoint.as_ref().is_some_and(|checkpoint| {
                    checkpoint.input_controller.pending_sequence.is_some()
                })
                || self.operations.has_device_pump())
        {
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ineligible {
                        reasons: vec![SnapshotIneligibleReason::SnapshotStateUnavailable],
                    },
                }),
                Some(message_id),
            );
        }
        if request.snapshot_purpose == SnapshotExportPurpose::Debug
            && self.active_debug_grant.is_none()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "debug snapshot export requires an active debug session",
            );
        }
        if request.kind == StateExportKind::CompiledProjectCache {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            if let Some(error) = self.compiled_cache_failure.clone() {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    &format!("compiled project cache preparation failed: {error}"),
                );
            }
            if self.compiled_project_cache.is_none() && self.compiled_cache_task.is_none() {
                if let Err(error) = self.start_compiled_cache_build() {
                    return self.reject(message_id, CommandErrorCode::ResourceLimit, &error);
                }
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "compiled project cache preparation started",
                );
            }
            let Some(bytes) = self.compiled_project_cache.clone() else {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    if self.compiled_cache_task.is_some() {
                        "compiled project cache is still being prepared"
                    } else {
                        "no compiled project cache is available"
                    },
                );
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "compiled project cache exceeds the negotiated transfer limit",
                );
            }
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: None,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes,
                next_offset: 0,
            });
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ready {
                        transfer: descriptor,
                    },
                }),
                Some(message_id),
            );
        }
        if request.kind == StateExportKind::FullProjectFile {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            if let Some(error) = self.full_project_failure.clone() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    &format!("full project preparation failed: {error}"),
                );
            }
            if self.full_project_file.is_none() && self.full_project_task.is_none() {
                if let Err(error) = self.start_full_project_build() {
                    return self.reject(message_id, CommandErrorCode::InvalidValue, &error);
                }
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "full project preparation started",
                );
            }
            let Some(bytes) = self.full_project_file.take() else {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "full project is still being prepared",
                );
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "full project exceeds the negotiated transfer limit",
                );
            }
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: None,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes,
                next_offset: 0,
            });
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ready {
                        transfer: descriptor,
                    },
                }),
                Some(message_id),
            );
        }
        if request.kind == StateExportKind::InputReplay {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            let bytes = self
                .input_replay
                .encode()
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "input replay exceeds the negotiated transfer limit",
                );
            }
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: None,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes: Arc::new(bytes),
                next_offset: 0,
            });
            return self.emit(
                RuntimeMessage::StateExportReady(StateExportReady {
                    kind: request.kind,
                    result: StateExportResult::Ready {
                        transfer: descriptor,
                    },
                }),
                Some(message_id),
            );
        }
        let unrestricted_snapshot = request.kind == StateExportKind::VmSnapshot
            && request.snapshot_purpose != SnapshotExportPurpose::Normal;
        let stable_wait = self.operations.active_input().is_some_and(|pending| {
            pending.wait.stability == WaitStability::StableInput
                && pending.wait.deadline_ns.is_none()
        });
        let mut reasons = Vec::new();
        if request.kind == StateExportKind::VmSnapshot
            && let Err(blocker) = self.sql.snapshot()
        {
            reasons.push(match blocker {
                crate::sql::SqlSnapshotBlocker::Inflight => {
                    SnapshotIneligibleReason::ExternalOperationPending
                }
                crate::sql::SqlSnapshotBlocker::Reader
                | crate::sql::SqlSnapshotBlocker::Transaction
                | crate::sql::SqlSnapshotBlocker::RevisionMissing => {
                    SnapshotIneligibleReason::SnapshotStateUnavailable
                }
            });
        }
        if !unrestricted_snapshot {
            if self.phase != RuntimePhase::WaitingInput || !stable_wait {
                reasons.push(SnapshotIneligibleReason::StableWaitRequired);
            }
            if self.operations.has_transient_external() || !self.effect_journal.is_empty() {
                reasons.push(SnapshotIneligibleReason::ExternalOperationPending);
            }
            if request.kind == StateExportKind::VmSnapshot && !self.operations.is_snapshot_stable()
            {
                reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
            }
            if request.kind == StateExportKind::VmSnapshot && self.undo_replay.is_some() {
                reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
            }
        }
        if request.kind == StateExportKind::VmSnapshot && self.vm.is_none() {
            reasons.push(SnapshotIneligibleReason::VmSnapshotUnavailable);
        }
        if request.kind == StateExportKind::VmSnapshot && self.project_snapshot.is_none() {
            reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
        }
        let result = if reasons.is_empty() {
            if self.outbound_transfer.is_some() {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "another state export is already active",
                );
            }
            let vm = self
                .vm
                .as_ref()
                .ok_or_else(|| RuntimeError::Internal("save export has no VM".into()))?;
            let bytes = match request.kind {
                StateExportKind::TraditionalSave => encode_era_save(
                    &vm.export_era_state(),
                    vm.vm().artifact(),
                    String::new(),
                    merge_structured_extensions(
                        &self.save_extensions,
                        vm.structured_extensions(StructuredScope::Ordinary)
                            .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                    )
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                    self.traditional_save_format(),
                )
                .map_err(|error| RuntimeError::Internal(error.to_string()))?,
                StateExportKind::VmSnapshot => {
                    if !unrestricted_snapshot
                        && !matches!(vm.snapshot_eligibility(), SnapshotEligibility::Eligible)
                    {
                        self.emit_log(
                            RuntimeLogLevel::Warning,
                            "state export is ineligible: SnapshotStateUnavailable",
                        )?;
                        return self.emit(
                            RuntimeMessage::StateExportReady(StateExportReady {
                                kind: request.kind,
                                result: StateExportResult::Ineligible {
                                    reasons: vec![
                                        SnapshotIneligibleReason::SnapshotStateUnavailable,
                                    ],
                                },
                            }),
                            Some(message_id),
                        );
                    }
                    let vm_snapshot = if unrestricted_snapshot {
                        vm.encode_unrestricted_snapshot()
                    } else {
                        vm.encode_snapshot()
                    }
                    .map_err(|error| {
                        RuntimeError::Internal(format!("VM snapshot encode failed: {error}"))
                    })?;
                    let project = self.project_snapshot.as_ref().ok_or_else(|| {
                        RuntimeError::Internal("snapshot export has no project identity".into())
                    })?;
                    runtime_snapshot::encode(&RuntimeSnapshotPayload {
                        format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
                        origin: match request.snapshot_purpose {
                            SnapshotExportPurpose::Normal => RuntimeSnapshotOrigin::Normal,
                            SnapshotExportPurpose::Debug => RuntimeSnapshotOrigin::Debug,
                            SnapshotExportPurpose::Diagnosis => RuntimeSnapshotOrigin::Diagnosis,
                        },
                        artifact_id: vm.artifact_id(),
                        compatibility: vm.vm().artifact().manifest.compatibility.clone(),
                        project_identity: project.project_identity,
                        resource_count: u64::try_from(project.resources.len()).unwrap_or(u64::MAX),
                        resource_graph: project.resource_graph.compact_snapshot(),
                        epoch: self.epoch.0,
                        vm_snapshot,
                        presentation: self.presentation.clone(),
                        operations: self.operations.clone(),
                        sql: self.sql.snapshot().map_err(|_| {
                            RuntimeError::Internal(
                                "SQL snapshot state changed after eligibility check".into(),
                            )
                        })?,
                        controller: self.controller.clone(),
                        logical_time_ns: self.logical_time_ns,
                        random_seed: self.random_seed,
                        selected_locale: self.selected_locale.clone(),
                        culture_table_version: CULTURE_TABLE_VERSION,
                        message_skip: self.message_skip,
                        skip_print: self.skip_print,
                        user_defined_skip: self.user_defined_skip,
                        saved_skip: self.saved_skip,
                        force_kana_mode: self.force_kana_mode,
                        hotkey_state: self.hotkey_state.clone(),
                        key_macros: self.key_macros.clone(),
                        input_controller: self.input_controller.clone(),
                        text_box: self.text_box.clone(),
                        text_box_layout: self.text_box_layout,
                        flow_input_enabled: self.flow_input_enabled,
                        flow_input_default: self.flow_input_default,
                        flow_input_can_skip: self.flow_input_can_skip,
                        flow_input_force_skip: self.flow_input_force_skip,
                        flow_input_string: self.flow_input_string,
                        flow_input_default_string: self.flow_input_default_string.clone(),
                        button_generation: self.button_generation,
                        debug_output: self.debug_output.clone(),
                        debug_output_base: self.debug_output_base,
                        command_intents: self.command_intents.clone(),
                        reusable_system_intents: self.reusable_system_intents.clone(),
                        save_extensions: self.save_extensions.clone(),
                        system_menu: match self.system_menu {
                            SystemMenuState::Title => 0,
                            SystemMenuState::LoadSlots => 1,
                            SystemMenuState::SaveSlots => 2,
                            SystemMenuState::ConfirmOverwrite { .. } => 3,
                        },
                        system_menu_slot: match self.system_menu {
                            SystemMenuState::ConfirmOverwrite { slot } => Some(slot),
                            _ => None,
                        },
                        load_slot_paths: self.load_slot_paths.clone(),
                        occupied_slot_paths: self.occupied_slot_paths.clone(),
                        system_menu_host_request: self.system_menu_host_request,
                        system_menu_page: self.system_menu_page,
                        undo_checkpoint: self.undo_checkpoint.clone(),
                        undo_replay: self.undo_replay.clone(),
                    })
                    .map_err(RuntimeError::Internal)?
                }
                StateExportKind::CompiledProjectCache
                | StateExportKind::FullProjectFile
                | StateExportKind::InputReplay
                | StateExportKind::FullProjectManifest => {
                    unreachable!("handled above")
                }
            };
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > self.options.limits.maximum_transfer_bytes
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::ResourceLimit,
                    "state export exceeds the negotiated transfer limit",
                );
            }
            let export_artifact_id = (request.kind == StateExportKind::VmSnapshot)
                .then(|| ProtocolBytes::new(vm.artifact_id().bytes()));
            let transfer_id = self.allocate_transfer();
            let descriptor = StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                digest: ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec()),
                artifact_id: export_artifact_id,
            };
            self.outbound_transfer = Some(OutboundStateTransfer {
                descriptor: descriptor.clone(),
                bytes: Arc::new(bytes),
                next_offset: 0,
            });
            StateExportResult::Ready {
                transfer: descriptor,
            }
        } else {
            self.emit_log(
                RuntimeLogLevel::Warning,
                format!("state export is ineligible: {reasons:?}"),
            )?;
            StateExportResult::Ineligible { reasons }
        };
        self.emit(
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: request.kind,
                result,
            }),
            Some(message_id),
        )
    }
}
