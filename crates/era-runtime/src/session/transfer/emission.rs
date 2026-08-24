#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn traditional_save_format(&self) -> era_runtime_save::SaveFormat {
        match self.project_snapshot.as_ref() {
            Some(snapshot) if snapshot.save_in_binary && snapshot.compress_save => {
                era_runtime_save::SaveFormat::Binary1808Gzip
            }
            Some(snapshot) if snapshot.save_in_binary => era_runtime_save::SaveFormat::Binary1808,
            _ => era_runtime_save::SaveFormat::Text1808,
        }
    }

    pub(in super::super) fn shutdown(&mut self, message_id: u64) -> Result<(), RuntimeError> {
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

    pub(in super::super) fn resynchronize(&mut self, message_id: u64) -> Result<(), RuntimeError> {
        self.materialize_resource_replay();
        let input_undo = self.input_undo_state();
        let presentation = self.presentation.snapshot_for_delivery();
        self.pending_presentation_update = false;
        self.emit(
            RuntimeMessage::RuntimeResynchronized(Box::new(RuntimeResynchronized {
                epoch: self.epoch.0,
                phase: self.phase,
                runtime_revision: self.revision,
                presentation,
                exit_requested: self.exit_requested,
                selected_locale: self.selected_locale.clone(),
                input_undo,
                key_macros: self.key_macros.state(),
                configuration: self
                    .project_snapshot
                    .as_ref()
                    .map(NormalizedProjectSnapshot::configuration_snapshot),
            })),
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

    #[allow(
        clippy::unnecessary_wraps,
        reason = "callers retain the fallible presentation-delivery contract; encoding now occurs at the drive boundary"
    )]
    pub(in super::super) fn emit_presentation(&mut self) -> Result<(), RuntimeError> {
        self.materialize_resource_replay_if_ready();
        self.pending_presentation_update = true;
        Ok(())
    }

    pub(in super::super) fn flush_presentation(&mut self) -> Result<(), RuntimeError> {
        if !self.pending_presentation_update {
            return Ok(());
        }
        // Reference Emuera drains consecutive skippable waits inside one secondary-click handler,
        // without returning to the platform message loop between frames. Keep the canonical model
        // current, but defer its projection while that same skip is still running so remote hosts
        // do not serialize and render every discarded animation frame.
        if self.phase == RuntimePhase::Running && self.message_skip {
            return Ok(());
        }
        self.publish_pending_presentation()
    }

    pub(in super::super) fn flush_presentation_for_observation(
        &mut self,
    ) -> Result<(), RuntimeError> {
        self.publish_pending_presentation()
    }

    fn publish_pending_presentation(&mut self) -> Result<(), RuntimeError> {
        if !self.pending_presentation_update {
            return Ok(());
        }
        self.materialize_resource_replay_if_ready();
        self.pending_presentation_update = false;
        let message = match self.presentation.next_update() {
            PresentationUpdate::Snapshot(snapshot) => {
                RuntimeMessage::PresentationSnapshot(snapshot)
            }
            PresentationUpdate::Delta(delta) => RuntimeMessage::PresentationDelta(delta),
        };
        self.emit_immediate(message, None)
    }

    pub(in super::super) fn sync_resource_replay(&mut self) -> bool {
        self.presentation.mark_resource_replay_stale();
        self.materialize_resource_replay_if_ready()
    }

    fn materialize_resource_replay_if_ready(&mut self) -> bool {
        if !self.presentation.resource_replay_is_ready_to_publish() {
            return false;
        }
        self.materialize_resource_replay();
        true
    }

    fn materialize_resource_replay(&mut self) {
        if !self.presentation.resource_replay_stale() {
            return;
        }
        let replay = self
            .project_snapshot
            .as_ref()
            .map(|project| project.resource_graph.replay())
            .unwrap_or_default();
        self.presentation.set_resource_replay(replay);
    }

    pub(in super::super) fn complete_graphics_result(
        &mut self,
        vm: &mut RuntimeVm,
        request: erabasic_vm::HostRequestId,
        value: i64,
    ) -> Result<(), RuntimeError> {
        commit_integer_result(vm, request, value)?;
        // Every caller reports zero only when the resource graph was left unchanged.
        if value == 0 {
            return Ok(());
        }
        if self.sync_resource_replay() {
            self.emit_presentation()
        } else {
            Ok(())
        }
    }

    pub(in super::super) fn set_phase(&mut self, phase: RuntimePhase) -> Result<(), RuntimeError> {
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

    pub(in super::super) fn reject(
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
        )?;
        let level = if message.starts_with("compiled project cache preparation")
            || message == "projection environment revision is not newer"
            || message == "projection observation does not match the canonical presentation"
        {
            RuntimeLogLevel::Debug
        } else {
            RuntimeLogLevel::Warning
        };
        self.emit_log(level, format!("command rejected [{code:?}]: {message}"))
    }

    pub(in super::super) fn fault(
        &mut self,
        code: FaultCode,
        message: &str,
        origin: Option<erabasic_vm::VmExecutionOrigin>,
    ) -> Result<(), RuntimeError> {
        self.emit_log(
            RuntimeLogLevel::Error,
            format!("runtime fault [{code:?}]: {message}"),
        )?;
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

    pub(in super::super) fn emit_log(
        &mut self,
        level: RuntimeLogLevel,
        message: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Log(RuntimeLog {
                level,
                message: message.into(),
            }),
            None,
        )
    }

    // Taking ownership prevents callers from accidentally retaining a message they
    // believe has been queued, even though encoding itself only borrows it.
    #[allow(clippy::needless_pass_by_value)]
    pub(in super::super) fn emit(
        &mut self,
        message: RuntimeMessage,
        correlation_id: Option<u64>,
    ) -> Result<(), RuntimeError> {
        // Presentation calls can occur hundreds of times while one VM slice constructs a
        // line. Serialize the authoritative result once at the caller boundary, but flush it
        // before any subsequent non-presentation message to preserve protocol ordering.
        self.flush_presentation()?;
        self.emit_immediate(message, correlation_id)
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the outbound journal takes sole ownership after encoding"
    )]
    fn emit_immediate(
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

    pub(in super::super) fn allocate_request(&mut self) -> Result<u64, RuntimeError> {
        if self.operations.total_count() >= self.options.limits.maximum_pending_requests as usize {
            return Err(RuntimeError::ResourceLimit(
                "too many pending service requests",
            ));
        }
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        Ok(id)
    }

    pub(in super::super) fn allocate_wait(&mut self) -> u64 {
        let id = self.next_wait_id;
        self.next_wait_id = self.next_wait_id.saturating_add(1);
        id
    }

    pub(in super::super) fn allocate_transfer(&mut self) -> u64 {
        let id = self.next_transfer_id;
        self.next_transfer_id = self.next_transfer_id.saturating_add(1);
        id
    }

    pub(in super::super) fn emit_effect(&mut self, kind: EffectKind) -> Result<(), RuntimeError> {
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

    pub(in super::super) fn emit_audio_unavailable(&mut self) -> Result<(), RuntimeError> {
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.audio_device_unavailable".into(),
                level: RuntimeLogLevel::Warning,
                message: "audio intent was retained but no frontend audio device is available"
                    .into(),
                source: None,
                notification: DiagnosticNotification::default(),
            }),
            None,
        )
    }

    pub(in super::super) fn acknowledge_effects(
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
                        level: RuntimeLogLevel::Warning,
                        message: outcome.message.unwrap_or_else(|| {
                            format!(
                                "frontend reported {:?} for effect {}",
                                outcome.status, outcome.effect_id
                            )
                        }),
                        source: None,
                        notification: DiagnosticNotification::default(),
                    }),
                    Some(message_id),
                )?;
            }
        }
        Ok(())
    }

    pub(in super::super) fn observe_frontend_time(&mut self, sample: u64) -> u64 {
        let (frontend_origin, logical_origin) = *self
            .frontend_time_origin
            .get_or_insert((sample, self.logical_time_ns));
        let mapped = logical_origin.saturating_add(sample.saturating_sub(frontend_origin));
        self.logical_time_ns = self.logical_time_ns.max(mapped);
        self.logical_time_ns
    }

    pub(in super::super) fn allocate_interaction(&mut self) -> InteractionToken {
        let token = InteractionToken {
            epoch: self.epoch.0,
            id: self.next_interaction_id,
        };
        self.next_interaction_id = self.next_interaction_id.saturating_add(1);
        token
    }

    pub(in super::super) fn advance_epoch(&mut self) {
        self.epoch.0 = self.epoch.0.saturating_add(1);
        self.operations.bind_epoch(self.epoch.0);
        self.command_intents.clear();
        self.reusable_system_intents.clear();
        self.next_interaction_id = 1;
        self.accepted_message_ids.clear();
        self.accepted_debug_message_ids.clear();
    }
}
