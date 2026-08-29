//! Reference-style Ctrl-Z input history and deterministic replay.

#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(super) fn input_undo_state(&mut self) -> InputUndoState {
        let enabled = self
            .negotiated_features
            .contains(&RuntimeFeature::InputUndo)
            && self
                .project_snapshot
                .as_ref()
                .is_some_and(|project| project.ctrl_z_enabled);
        let available_steps = self.undo_checkpoint.as_ref().map_or(0, |checkpoint| {
            u32::try_from(checkpoint.inputs.len()).unwrap_or(u32::MAX)
        });
        let available = enabled && available_steps != 0;
        if !available {
            self.undo_token = None;
        } else if self.undo_token.is_none() {
            self.undo_token = Some(self.allocate_interaction());
        }
        InputUndoState {
            enabled,
            available_steps,
            in_progress: self.undo_replay.is_some(),
            runtime_revision: self.revision,
            token: self.undo_token,
        }
    }

    pub(super) fn emit_input_undo_state(&mut self) -> Result<(), RuntimeError> {
        let state = self.input_undo_state();
        self.emit(RuntimeMessage::InputUndoStateChanged(state), None)
    }

    pub(super) fn invalidate_input_undo(
        &mut self,
        reason: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let invalidated = self.undo_checkpoint.is_some() || self.undo_replay.is_some();
        self.undo_checkpoint = None;
        self.undo_replay = None;
        self.undo_token = None;
        if let Some(message) = reason.filter(|_| invalidated) {
            self.emit(
                RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                    context: None,
                    code: "runtime.input_undo_invalidated".into(),
                    level: RuntimeLogLevel::Warning,
                    message: message.into(),
                    source: None,
                    notification: DiagnosticNotification::default(),
                }),
                None,
            )?;
        }
        self.emit_input_undo_state()
    }

    pub(super) fn establish_input_undo_checkpoint(
        &mut self,
        slot: u32,
        save_bytes: Vec<u8>,
        random_state: Vec<i64>,
    ) -> Result<(), RuntimeError> {
        self.undo_checkpoint = Some(UndoCheckpoint {
            slot,
            save_bytes,
            random_state,
            inputs: Vec::new(),
            input_history_bytes: 0,
            input_controller: self.input_controller.clone(),
        });
        self.undo_replay = None;
        self.undo_token = None;
        self.emit_input_undo_state()
    }

    pub(super) fn record_input_undo_value(&mut self, value: &VmValue) -> Result<(), RuntimeError> {
        if self.undo_replay.is_some() {
            return Ok(());
        }
        let Some(checkpoint) = self.undo_checkpoint.as_mut() else {
            return Ok(());
        };
        let value = match value {
            VmValue::Integer(value) => value.to_string(),
            VmValue::String(value) => value.clone(),
            VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => return Ok(()),
        };
        if checkpoint.inputs.len() >= self.options.limits.maximum_journal_entries as usize {
            return self.invalidate_input_undo(Some(
                "Ctrl-Z input history exceeded the negotiated journal limit",
            ));
        }
        let record = RecordedInput {
            value,
            source: self.active_input_source.clone(),
        };
        let Some(bytes) = record
            .storage_bytes()
            .and_then(|bytes| checkpoint.input_history_bytes.checked_add(bytes))
            .filter(|bytes| *bytes <= self.options.limits.maximum_transfer_bytes)
        else {
            return self.invalidate_input_undo(Some(
                "Ctrl-Z input provenance exceeded the negotiated transfer limit",
            ));
        };
        checkpoint.input_history_bytes = bytes;
        checkpoint.inputs.push(record);
        self.undo_token = None;
        self.emit_input_undo_state()
    }

    pub(super) fn request_input_undo(
        &mut self,
        message_id: u64,
        request: &InputUndoRequest,
    ) -> Result<(), RuntimeError> {
        if !self
            .negotiated_features
            .contains(&RuntimeFeature::InputUndo)
        {
            return self.reject(
                message_id,
                CommandErrorCode::FeatureUnavailable,
                "input undo was not negotiated",
            );
        }
        if self.undo_token != Some(request.token) || request.token.epoch != self.epoch.0 {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "input undo token is stale",
            );
        }
        if self.undo_replay.is_some() {
            if let Some(replay) = self.undo_replay.as_mut() {
                replay.queued_repeats = replay.queued_repeats.saturating_add(1);
            }
            self.undo_token = None;
            return self.emit_input_undo_state();
        }
        if self.phase != RuntimePhase::WaitingInput
            || self.pending_candidate_commit.is_some()
            || self.operations.has_candidate_write()
            || self.input_controller.pending_sequence.is_some()
            || self.operations.has_device_pump()
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "input undo requires a stable input wait",
            );
        }
        let Some(checkpoint) = self.undo_checkpoint.as_mut() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "input undo has no save/load checkpoint",
            );
        };
        let Some(removed) = checkpoint.inputs.pop() else {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "input undo history is empty",
            );
        };
        checkpoint.input_history_bytes = checkpoint
            .input_history_bytes
            .saturating_sub(removed.storage_bytes().unwrap_or(u64::MAX));
        let slot = checkpoint.slot;
        let bytes = checkpoint.save_bytes.clone();
        let random = checkpoint.random_state.clone();
        let remaining = checkpoint.inputs.iter().cloned().collect();
        self.undo_replay = Some(UndoReplay {
            remaining,
            queued_repeats: 0,
        });
        self.undo_token = None;
        if let Some(wait) = self.operations.take_active_input() {
            self.close_wait(wait.wait.wait_id)?;
        }
        self.operations.clear();
        self.effect_journal.clear();
        self.reset_input_undo_presentation();
        self.vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("input undo has no VM".into()))?
            .restore_random_state(&random)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.emit_input_undo_state()?;
        self.complete_ordinary_load(slot, &bytes)
    }

    pub(super) fn replay_submission(
        &mut self,
        pending: &PendingInput,
    ) -> Result<Option<InputSubmission>, RuntimeError> {
        let Some(replay) = self.undo_replay.as_ref() else {
            return Ok(None);
        };
        let Some(record) = replay.remaining.front().cloned() else {
            self.undo_replay = None;
            self.undo_token = None;
            return Ok(None);
        };
        if pending.wait.kind == WaitKind::PrimitiveMouseKey {
            return Ok(None);
        }
        if let Some(source) = record.source {
            match source.root {
                InputRoot::Sequence(_) => return Ok(None),
                InputRoot::External => {
                    if self.input_controller.pending_sequence.is_some() {
                        return Ok(None); // Let the regenerated script input run first; finish verifies provenance.
                    }
                    if let Some(queued) = self.queued_input.front() {
                        if !queued.source.same_replay_origin(&source) {
                            return Err(RuntimeError::Internal(
                                "replay queue source differs from recorded input".into(),
                            ));
                        }
                        return Ok(None);
                    }
                    if source.macro_enabled != self.input_controller.macro_enabled {
                        return Err(RuntimeError::Internal(
                            "replay macro switch differs from recorded admission".into(),
                        ));
                    }
                    let regenerated = self
                        .input_controller
                        .admit(
                            InputRoot::External,
                            source.raw.as_ref().clone(),
                            source.message_skip,
                        )
                        .map_err(RuntimeError::ResourceLimit)?;
                    return self.prepare_text_fragment(pending, &regenerated, source.fragment);
                }
            }
        }
        self.active_input_source = None;
        let value =
            match pending.wait.kind {
                WaitKind::StringValue | WaitKind::StringButton => VmValue::String(record.value),
                _ => VmValue::Integer(record.value.trim().parse().map_err(|_| {
                    RuntimeError::Internal("replay value is not an integer".into())
                })?),
            };
        Ok(Some(InputSubmission::Value(value)))
    }

    pub(super) fn restart_queued_input_undo(&mut self) -> Result<bool, RuntimeError> {
        let should_restart = self
            .undo_replay
            .as_ref()
            .is_some_and(|replay| replay.remaining.is_empty() && replay.queued_repeats != 0);
        if !should_restart {
            return Ok(false);
        }
        let checkpoint = self.undo_checkpoint.as_mut().ok_or_else(|| {
            RuntimeError::Internal("queued input undo lost its checkpoint".into())
        })?;
        let Some(removed) = checkpoint.inputs.pop() else {
            if let Some(replay) = self.undo_replay.as_mut() {
                replay.queued_repeats = 0;
            }
            return Ok(false);
        };
        checkpoint.input_history_bytes = checkpoint
            .input_history_bytes
            .saturating_sub(removed.storage_bytes().unwrap_or(u64::MAX));
        let slot = checkpoint.slot;
        let bytes = checkpoint.save_bytes.clone();
        let random = checkpoint.random_state.clone();
        let remaining = checkpoint.inputs.iter().cloned().collect();
        let queued_repeats = self
            .undo_replay
            .as_ref()
            .map_or(0, |replay| replay.queued_repeats.saturating_sub(1));
        self.undo_replay = Some(UndoReplay {
            remaining,
            queued_repeats,
        });
        self.undo_token = None;
        self.operations.clear();
        self.effect_journal.clear();
        self.reset_input_undo_presentation();
        self.vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("queued input undo has no VM".into()))?
            .restore_random_state(&random)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.emit_input_undo_state()?;
        self.complete_ordinary_load(slot, &bytes)?;
        Ok(true)
    }

    fn reset_input_undo_presentation(&mut self) {
        self.presentation.reset_preserving_projection();
        self.pending_presentation_update = false;
        if let Some(project) = &self.project_snapshot {
            self.presentation.configure_project(project);
        }
    }
}
