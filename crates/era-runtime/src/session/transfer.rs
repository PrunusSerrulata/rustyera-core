//! State transfer, shutdown, projection, and protocol emission utilities.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(super) fn export_state(
        &mut self,
        message_id: u64,
        request: StateExportRequest,
    ) -> Result<(), RuntimeError> {
        let stable_wait = self.operations.active_input().is_some_and(|pending| {
            pending.wait.stability == WaitStability::StableInput
                && pending.wait.deadline_ns.is_none()
        });
        let mut reasons = Vec::new();
        if self.phase != RuntimePhase::WaitingInput || !stable_wait {
            reasons.push(SnapshotIneligibleReason::StableWaitRequired);
        }
        if self.operations.has_transient_external() || !self.effect_journal.is_empty() {
            reasons.push(SnapshotIneligibleReason::ExternalOperationPending);
        }
        if request.kind == StateExportKind::VmSnapshot && !self.operations.is_snapshot_stable() {
            reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
        }
        if request.kind == StateExportKind::VmSnapshot && self.undo_replay.is_some() {
            reasons.push(SnapshotIneligibleReason::SnapshotStateUnavailable);
        }
        if request.kind == StateExportKind::VmSnapshot && !self.queued_input.is_empty() {
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
                    let vm_snapshot = match vm.snapshot() {
                        Ok(snapshot) => snapshot.encode().map_err(|error| {
                            RuntimeError::Internal(format!("VM snapshot encode failed: {error}"))
                        })?,
                        Err(_) => {
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
                    };
                    let project = self.project_snapshot.as_ref().ok_or_else(|| {
                        RuntimeError::Internal("snapshot export has no project identity".into())
                    })?;
                    runtime_snapshot::encode(&RuntimeSnapshotPayload {
                        format_version: RUNTIME_SNAPSHOT_FORMAT_VERSION,
                        artifact_id: vm.artifact_id(),
                        project_identity: project.project_identity,
                        resource_count: u64::try_from(project.resources.len()).unwrap_or(u64::MAX),
                        resource_graph: project.resource_graph.clone(),
                        epoch: self.epoch.0,
                        vm_snapshot,
                        presentation: self.presentation.clone(),
                        operations: self.operations.clone(),
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
                bytes,
                next_offset: 0,
            });
            StateExportResult::Ready {
                transfer: descriptor,
            }
        } else {
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

    pub(super) fn begin_state_import(
        &mut self,
        message_id: u64,
        request: StateImportBegin,
    ) -> Result<(), RuntimeError> {
        if self.inbound_transfer.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "another state import is already active",
            );
        }
        if request.total_bytes > self.options.limits.maximum_transfer_bytes {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import exceeds the negotiated transfer limit",
            );
        }
        if request.digest.as_slice().len() != blake3::OUT_LEN {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import digest must contain 32 bytes",
            );
        }
        match usize::try_from(request.total_bytes) {
            Ok(_) => {}
            Err(_) => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "state import length is not addressable on this platform",
                );
            }
        }
        let transfer_id = self.allocate_transfer();
        self.inbound_transfer = Some(InboundStateTransfer {
            descriptor: StateTransferDescriptor {
                transfer_id,
                kind: request.kind,
                total_bytes: request.total_bytes,
                digest: request.digest,
                artifact_id: request.artifact_id,
            },
            // Grow with accepted chunks instead of trusting a potentially huge declaration.
            bytes: Vec::new(),
            committed: false,
        });
        self.emit(
            RuntimeMessage::StateImportAccepted(StateImportAccepted { transfer_id }),
            Some(message_id),
        )
    }

    pub(super) fn append_state_import(
        &mut self,
        message_id: u64,
        chunk: &StateImportChunk,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state import is active",
            );
        };
        if transfer.descriptor.transfer_id != chunk.transfer_id || transfer.committed {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state import transfer is stale",
            );
        }
        if chunk.offset != u64::try_from(transfer.bytes.len()).unwrap_or(u64::MAX) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import chunks must be contiguous and ordered",
            );
        }
        if chunk.data.as_slice().is_empty()
            || chunk
                .offset
                .saturating_add(u64::try_from(chunk.data.as_slice().len()).unwrap_or(u64::MAX))
                > transfer.descriptor.total_bytes
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import chunk has an invalid length",
            );
        }
        transfer
            .bytes
            .try_reserve(chunk.data.as_slice().len())
            .map_err(|_| RuntimeError::ResourceLimit("state import allocation failed"))?;
        transfer.bytes.extend_from_slice(chunk.data.as_slice());
        Ok(())
    }

    pub(super) fn commit_state_import(
        &mut self,
        message_id: u64,
        commit: StateImportCommit,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state import is active",
            );
        };
        if transfer.descriptor.transfer_id != commit.transfer_id || transfer.committed {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state import transfer is stale",
            );
        }
        if u64::try_from(transfer.bytes.len()).unwrap_or(u64::MAX)
            != transfer.descriptor.total_bytes
            || transfer.descriptor.digest.as_slice() != blake3::hash(&transfer.bytes).as_bytes()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import length or digest does not match its descriptor",
            );
        }
        transfer.committed = true;
        let kind = transfer.descriptor.kind;
        self.emit(
            RuntimeMessage::StateImportReady(StateImportReady {
                transfer_id: commit.transfer_id,
                kind,
            }),
            Some(message_id),
        )
    }

    pub(super) fn read_state_export(
        &mut self,
        message_id: u64,
        request: StateExportChunkRequest,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.outbound_transfer.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "no state export is active",
            );
        };
        if transfer.descriptor.transfer_id != request.transfer_id {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state export transfer is stale",
            );
        }
        if request.offset != transfer.next_offset {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state export chunks must be read contiguously and in order",
            );
        }
        let offset = match usize::try_from(request.offset) {
            Ok(offset) if offset <= transfer.bytes.len() => offset,
            _ => {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "state export offset is outside the payload",
                );
            }
        };
        if request.maximum_bytes == 0 {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state export chunk size must be non-zero",
            );
        }
        let protocol_overhead = 1024_u64;
        let negotiated = self
            .options
            .limits
            .maximum_payload_bytes
            .saturating_sub(protocol_overhead);
        let requested = u64::from(request.maximum_bytes).min(negotiated);
        if requested == 0 {
            return self.reject(
                message_id,
                CommandErrorCode::ResourceLimit,
                "negotiated payload limit cannot carry a state chunk",
            );
        }
        let end = offset
            .saturating_add(usize::try_from(requested).unwrap_or(usize::MAX))
            .min(transfer.bytes.len());
        let complete = end == transfer.bytes.len();
        let response = StateExportChunk {
            transfer_id: request.transfer_id,
            offset: request.offset,
            data: ProtocolBytes::new(transfer.bytes[offset..end].to_vec()),
            complete,
        };
        self.emit(RuntimeMessage::StateExportChunk(response), Some(message_id))?;
        if complete {
            self.outbound_transfer = None;
        } else if let Some(transfer) = self.outbound_transfer.as_mut() {
            transfer.next_offset = u64::try_from(end).unwrap_or(u64::MAX);
        }
        Ok(())
    }

    pub(super) fn cancel_state_transfer(
        &mut self,
        message_id: u64,
        cancel: StateTransferCancel,
    ) -> Result<(), RuntimeError> {
        let inbound = self
            .inbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.transfer_id == cancel.transfer_id);
        let outbound = self
            .outbound_transfer
            .as_ref()
            .is_some_and(|transfer| transfer.descriptor.transfer_id == cancel.transfer_id);
        if !inbound && !outbound {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "state transfer is stale",
            );
        }
        if inbound {
            self.inbound_transfer = None;
        }
        if outbound {
            self.outbound_transfer = None;
        }
        Ok(())
    }

    pub(super) fn consume_state_import(
        &mut self,
        message_id: u64,
        transfer_id: u64,
        kind: StateExportKind,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        let valid = self.inbound_transfer.as_ref().is_some_and(|transfer| {
            transfer.descriptor.transfer_id == transfer_id
                && transfer.descriptor.kind == kind
                && transfer.committed
        });
        if !valid {
            self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "start requires a committed state import of the requested kind",
            )?;
            return Ok(None);
        }
        Ok(self.inbound_transfer.take().map(|transfer| transfer.bytes))
    }

    pub(super) fn traditional_save_format(&self) -> era_runtime_save::SaveFormat {
        match self.project_snapshot.as_ref() {
            Some(snapshot) if snapshot.save_in_binary && snapshot.compress_save => {
                era_runtime_save::SaveFormat::Binary1808Gzip
            }
            Some(snapshot) if snapshot.save_in_binary => era_runtime_save::SaveFormat::Binary1808,
            _ => era_runtime_save::SaveFormat::Text1808,
        }
    }

    pub(super) fn shutdown(&mut self, message_id: u64) -> Result<(), RuntimeError> {
        if self.operations.has_candidate_write() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "shutdown cannot cancel a candidate save after its atomic write was emitted",
            );
        }
        self.set_phase(RuntimePhase::Stopping)?;
        let cancelled = self
            .operations
            .total_count()
            .saturating_add(self.effect_journal.len());
        let (service_requests, storage_requests) = self.operations.external_requests();
        for request_id in service_requests {
            self.emit(
                RuntimeMessage::CancelExternalRequest(CancelExternalRequest {
                    request_id,
                    kind: ExternalRequestKind::Service,
                }),
                None,
            )?;
        }
        for request_id in storage_requests {
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
        self.set_phase(RuntimePhase::Stopped)?;
        self.emit(
            RuntimeMessage::ShutdownReady(ShutdownReady {
                final_runtime_revision: self.revision,
                pending_operations_cancelled: u32::try_from(cancelled).unwrap_or(u32::MAX),
            }),
            Some(message_id),
        )
    }

    pub(super) fn resynchronize(&mut self, message_id: u64) -> Result<(), RuntimeError> {
        let input_undo = self.input_undo_state();
        self.emit(
            RuntimeMessage::RuntimeResynchronized(RuntimeResynchronized {
                epoch: self.epoch.0,
                phase: self.phase,
                runtime_revision: self.revision,
                presentation: self.presentation.snapshot(),
                exit_requested: self.exit_requested,
                selected_locale: self.selected_locale.clone(),
                input_undo,
                key_macros: self.key_macros.state(),
            }),
            Some(message_id),
        )?;
        if !self.effect_journal.is_empty() {
            self.emit(
                RuntimeMessage::EffectBatch(EffectBatch {
                    effects: self.effect_journal.values().cloned().collect(),
                }),
                Some(message_id),
            )?;
        }
        Ok(())
    }

    pub(super) fn emit_presentation(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::PresentationSnapshot(self.presentation.snapshot()),
            None,
        )
    }

    pub(super) fn sync_resource_replay(&mut self) {
        let replay = self
            .project_snapshot
            .as_ref()
            .map(|project| project.resource_graph.replay())
            .unwrap_or_default();
        self.presentation.set_resource_replay(replay);
    }

    pub(super) fn complete_graphics_result(
        &mut self,
        vm: &mut RuntimeVm,
        request: erabasic_vm::HostRequestId,
        value: i64,
    ) -> Result<(), RuntimeError> {
        commit_integer_result(vm, request, value)?;
        self.sync_resource_replay();
        self.emit_presentation()
    }

    pub(super) fn set_phase(&mut self, phase: RuntimePhase) -> Result<(), RuntimeError> {
        self.phase = phase;
        self.revision = self.revision.saturating_add(1);
        self.emit(
            RuntimeMessage::StateChanged(RuntimeStateChanged {
                phase,
                revision: self.revision,
                epoch: self.epoch.0,
            }),
            None,
        )
    }

    pub(super) fn reject(
        &mut self,
        correlation_id: u64,
        code: CommandErrorCode,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::CommandRejected(CommandRejected {
                code,
                message: message.into(),
                recoverable: true,
                source: None,
            }),
            (correlation_id != 0).then_some(correlation_id),
        )
    }

    pub(super) fn fault(
        &mut self,
        code: FaultCode,
        message: &str,
        origin: Option<erabasic_vm::VmExecutionOrigin>,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Fault(RuntimeFault {
                code,
                message: message.into(),
                origin: origin.map(protocol_execution_origin),
            }),
            None,
        )?;
        self.set_phase(RuntimePhase::Faulted)
    }

    // Taking ownership prevents callers from accidentally retaining a message they
    // believe has been queued, even though encoding itself only borrows it.
    #[allow(clippy::needless_pass_by_value)]
    pub(super) fn emit(
        &mut self,
        message: RuntimeMessage,
        correlation_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        let envelope = message.envelope(
            Some(self.options.session_id),
            Some(self.epoch),
            self.outbound_sequence,
            self.next_message_id,
            correlation_id,
        )?;
        let bytes = encode_envelope(&envelope, self.options.wire_limits)?;
        if self.outbound_journal.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("outbound journal is full"));
        }
        self.outbound.push_back(bytes.clone());
        self.outbound_journal.insert(self.outbound_sequence, bytes);
        self.outbound_sequence = self.outbound_sequence.saturating_add(1);
        self.next_message_id = self.next_message_id.saturating_add(1);
        Ok(())
    }

    pub(super) fn allocate_request(&mut self) -> Result<u64, RuntimeError> {
        if self.operations.total_count() >= self.options.limits.maximum_pending_requests as usize {
            return Err(RuntimeError::ResourceLimit(
                "too many pending service requests",
            ));
        }
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        Ok(id)
    }

    pub(super) fn allocate_wait(&mut self) -> u64 {
        let id = self.next_wait_id;
        self.next_wait_id = self.next_wait_id.saturating_add(1);
        id
    }

    pub(super) fn allocate_transfer(&mut self) -> u64 {
        let id = self.next_transfer_id;
        self.next_transfer_id = self.next_transfer_id.saturating_add(1);
        id
    }

    pub(super) fn emit_effect(&mut self, kind: EffectKind) -> Result<(), RuntimeError> {
        if self.effect_journal.len() >= self.options.limits.maximum_journal_entries as usize {
            return Err(RuntimeError::ResourceLimit("effect journal is full"));
        }
        let event = EffectEvent {
            effect_id: self.next_effect_id,
            kind,
        };
        self.next_effect_id = self.next_effect_id.saturating_add(1);
        self.effect_journal.insert(event.effect_id, event.clone());
        self.emit(
            RuntimeMessage::EffectBatch(EffectBatch {
                effects: vec![event],
            }),
            None,
        )
    }

    pub(super) fn emit_audio_unavailable(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.audio_device_unavailable".into(),
                severity: DiagnosticSeverity::Warning,
                message: "audio intent was retained but no frontend audio device is available"
                    .into(),
                source: None,
            }),
            None,
        )
    }

    pub(super) fn acknowledge_effects(
        &mut self,
        message_id: u64,
        acknowledgement: EffectAcknowledgement,
    ) -> Result<(), RuntimeError> {
        let mut seen = BTreeSet::new();
        for outcome in &acknowledgement.outcomes {
            if !seen.insert(outcome.effect_id)
                || !self.effect_journal.contains_key(&outcome.effect_id)
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "effect acknowledgement refers to an unknown or duplicate effect",
                );
            }
        }
        for outcome in acknowledgement.outcomes {
            self.effect_journal.remove(&outcome.effect_id);
            if outcome.status != EffectOutcomeStatus::Completed {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.device_effect_failed".into(),
                        severity: DiagnosticSeverity::Warning,
                        message: outcome.message.unwrap_or_else(|| {
                            format!(
                                "frontend reported {:?} for effect {}",
                                outcome.status, outcome.effect_id
                            )
                        }),
                        source: None,
                    }),
                    Some(message_id),
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn observe_frontend_time(&mut self, sample: u64) -> u64 {
        let (frontend_origin, logical_origin) = *self
            .frontend_time_origin
            .get_or_insert((sample, self.logical_time_ns));
        let mapped = logical_origin.saturating_add(sample.saturating_sub(frontend_origin));
        self.logical_time_ns = self.logical_time_ns.max(mapped);
        self.logical_time_ns
    }

    pub(super) fn allocate_interaction(&mut self) -> InteractionToken {
        let token = InteractionToken {
            epoch: self.epoch.0,
            id: self.next_interaction_id,
        };
        self.next_interaction_id = self.next_interaction_id.saturating_add(1);
        token
    }

    pub(super) fn advance_epoch(&mut self) {
        self.epoch.0 = self.epoch.0.saturating_add(1);
        self.operations.bind_epoch(self.epoch.0);
        self.command_intents.clear();
        self.reusable_system_intents.clear();
        self.next_interaction_id = 1;
        self.accepted_message_ids.clear();
        self.accepted_debug_message_ids.clear();
    }
}
