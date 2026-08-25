// This is part of the split RuntimeSession storage implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn begin_candidate_save(
        &mut self,
        _vm: &mut RuntimeVm,
        slot: u32,
        continuation: CandidateSaveContinuation,
    ) -> Result<(), RuntimeError> {
        let capabilities = self.storage_capabilities;
        if !(capabilities.revisions
            && capabilities.atomic_replace
            && capabilities.missing_precondition)
        {
            return match continuation {
                CandidateSaveContinuation::Autosave => {
                    self.emit(
                        RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                            code: "runtime.candidate_save_failed".into(),
                            level: RuntimeLogLevel::Warning,
                            message:
                                "frontend storage cannot provide revision-checked atomic writes"
                                    .into(),
                            source: None,
                            notification: DiagnosticNotification::default(),
                        }),
                        None,
                    )?;
                    self.stage_builtin_autosave_failure()
                }
                CandidateSaveContinuation::SystemMenu { .. } => self.finish_candidate_save_failure(
                    continuation,
                    "frontend storage cannot provide revision-checked atomic writes",
                ),
            };
        }
        self.issue_storage(
            PendingStorage::CandidateSaveStat { slot, continuation },
            StorageNamespace::Save,
            StorageOperation::Stat,
            save_slot_path(slot),
        )
    }

    pub(in super::super) fn begin_system_menu_candidate(
        &mut self,
        slot: u32,
    ) -> Result<(), RuntimeError> {
        let request = self.system_menu_host_request.ok_or_else(|| {
            RuntimeError::Internal("system save menu lost its VM continuation".into())
        })?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("system save menu has no VM".into()))?;
        let result = self.begin_candidate_save(
            &mut vm,
            slot,
            CandidateSaveContinuation::SystemMenu { request, slot },
        );
        self.vm = Some(vm);
        result
    }

    pub(in super::super) fn issue_candidate_clock(
        &mut self,
        slot: u32,
        precondition: StoragePrecondition,
        continuation: CandidateSaveContinuation,
    ) -> Result<(), RuntimeError> {
        if self
            .service_capabilities
            .get(&(ServiceKind::Clock, LOCAL_DATE_TIME_OPERATION.to_owned()))
            != Some(&LOCAL_DATE_TIME_OPERATION_VERSION)
        {
            return self.finish_candidate_save_failure(
                continuation,
                "frontend did not negotiate the candidate-save clock service",
            );
        }
        let request_id = self.allocate_request()?;
        self.operations.insert_service(
            request_id,
            PendingService::CandidateSaveClock {
                slot,
                precondition,
                continuation,
            },
        );
        self.emit(
            RuntimeMessage::ServiceRequest(ServiceRequest {
                request_id,
                kind: ServiceKind::Clock,
                operation: LOCAL_DATE_TIME_OPERATION.into(),
                operation_version: LOCAL_DATE_TIME_OPERATION_VERSION,
                payload: ProtocolBytes::new(encode_canonical(&LocalDateTimeRequest {})?),
                deadline_ns: None,
            }),
            None,
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn prepare_candidate_save(
        &mut self,
        time: LocalDateTimeResponse,
    ) -> Result<(PendingCandidateCommit, Vec<u8>), RuntimeError> {
        let mut candidate = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("candidate save has no VM".into()))?
            .fork_isolated()
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        write_runtime_string(
            &mut candidate,
            "SAVEDATA_TEXT",
            format!(
                "{:04}/{:02}/{:02} {:02}:{:02}:{:02} ",
                time.year, time.month, time.day, time.hour, time.minute, time.second
            ),
        )?;

        let function = candidate
            .vm()
            .artifact()
            .functions
            .iter()
            .find(|function| function.name.eq_ignore_ascii_case("SAVEINFO"))
            .map(|function| function.key);

        let original_presentation = self.presentation.clone();
        let original_project = self.project_snapshot.clone();
        let original_flags = (
            self.message_skip,
            self.skip_print,
            self.user_defined_skip,
            self.saved_skip,
            self.force_kana_mode,
        );
        let original_phase = self.phase;
        let original_revision = self.revision;
        let original_outbound = std::mem::take(&mut self.outbound);
        let original_outbound_journal = std::mem::take(&mut self.outbound_journal);
        let original_outbound_journal_bytes = std::mem::take(&mut self.outbound_journal_bytes);
        let original_effect_journal = std::mem::take(&mut self.effect_journal);
        let original_pending_presentation = self.pending_presentation_update;
        let original_projection_state = self.last_projection_state.clone();
        let original_sequence = self.outbound_sequence;
        let original_message = self.next_message_id;
        let original_effect = self.next_effect_id;
        self.candidate_clock = Some(time);
        let mut candidate_diagnostics = Vec::new();

        let execution = (|| -> Result<(), RuntimeError> {
            let Some(function) = function else {
                return Ok(());
            };
            let fiber = candidate
                .spawn_entry(function, Vec::new())
                .map_err(|error| RuntimeError::Internal(error.to_string()))?;
            let maximum = self.options.limits.maximum_drive_instructions.max(1);
            let mut executed = 0_u64;
            loop {
                synchronize_line_count(&mut self.presentation, &mut candidate)?;
                let report = candidate.drive(
                    RunBudget {
                        maximum_instructions: maximum.saturating_sub(executed).max(1),
                        maximum_host_calls: self.options.limits.maximum_pending_requests,
                        fiber_quantum: RunBudget::default().fiber_quantum,
                    },
                    VmDriveMode::Normal,
                );
                executed = executed.saturating_add(report.instructions);
                let mut completed = false;
                for event in report.events {
                    match event {
                        VmPortEvent::Diagnostic {
                            code,
                            message,
                            origin,
                            notification,
                            ..
                        } => candidate_diagnostics.push(ProtocolDiagnostic {
                            code,
                            level: RuntimeLogLevel::Warning,
                            message,
                            source: protocol_execution_origin(origin).source,
                            notification: protocol_diagnostic_notification(notification),
                        }),
                        VmPortEvent::HostCall(request) => {
                            self.handle_host_call(&mut candidate, &request)?;
                        }
                        VmPortEvent::FiberCompleted(id, _) if id == fiber => completed = true,
                        VmPortEvent::FiberFaulted(id, fault) if id == fiber => {
                            return Err(RuntimeError::Internal(format!(
                                "candidate SAVEINFO faulted: {}",
                                fault.message
                            )));
                        }
                        VmPortEvent::FiberYielded(_)
                        | VmPortEvent::DebugStopped(_)
                        | VmPortEvent::FiberCompleted(_, _)
                        | VmPortEvent::FiberFaulted(_, _) => {}
                    }
                }
                candidate.retire_terminal_fibers();
                if completed {
                    return Ok(());
                }
                if executed >= maximum {
                    return Err(RuntimeError::ResourceLimit(
                        "candidate SAVEINFO exceeded its instruction budget",
                    ));
                }
                if !candidate.has_runnable_fibers() {
                    return Err(RuntimeError::Internal(
                        "candidate SAVEINFO attempted to suspend".into(),
                    ));
                }
            }
        })();

        self.candidate_clock = None;
        let candidate_presentation = self.presentation.clone();
        let candidate_project = self.project_snapshot.clone();
        let candidate_flags = (
            self.message_skip,
            self.skip_print,
            self.user_defined_skip,
            self.saved_skip,
            self.force_kana_mode,
        );
        let effects = self
            .effect_journal
            .values()
            .map(|event| event.kind.clone())
            .collect();
        self.presentation = original_presentation;
        self.pending_presentation_update = original_pending_presentation;
        self.last_projection_state = original_projection_state;
        self.project_snapshot = original_project;
        (
            self.message_skip,
            self.skip_print,
            self.user_defined_skip,
            self.saved_skip,
            self.force_kana_mode,
        ) = original_flags;
        self.phase = original_phase;
        self.revision = original_revision;
        self.outbound = original_outbound;
        self.outbound_journal = original_outbound_journal;
        self.outbound_journal_bytes = original_outbound_journal_bytes;
        debug_assert_eq!(
            self.outbound_journal_bytes,
            self.outbound_journal
                .values()
                .map(|bytes| u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                .sum::<u64>()
        );
        self.effect_journal = original_effect_journal;
        self.outbound_sequence = original_sequence;
        self.next_message_id = original_message;
        self.next_effect_id = original_effect;
        for diagnostic in candidate_diagnostics {
            self.emit(RuntimeMessage::Diagnostic(diagnostic), None)?;
        }
        execution?;

        let description = read_runtime_string(&candidate, "SAVEDATA_TEXT")?;
        let bytes = encode_scoped_save(
            &candidate.export_era_state(),
            candidate.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
            description,
            merge_structured_extensions(
                &self.save_extensions,
                candidate
                    .structured_extensions(StructuredScope::Ordinary)
                    .map_err(|error| RuntimeError::Internal(error.to_string()))?,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?,
            self.traditional_save_format(),
        )
        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        Ok((
            PendingCandidateCommit {
                state: candidate.into_candidate_state(),
                presentation: candidate_presentation,
                project_snapshot: candidate_project,
                message_skip: candidate_flags.0,
                skip_print: candidate_flags.1,
                user_defined_skip: candidate_flags.2,
                saved_skip: candidate_flags.3,
                force_kana_mode: candidate_flags.4,
                effects,
                save_bytes: Vec::new(),
                save_slot: None,
            },
            bytes,
        ))
    }

    pub(in super::super) fn finish_candidate_save_failure(
        &mut self,
        continuation: CandidateSaveContinuation,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.pending_candidate_commit = None;
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.candidate_save_failed".into(),
                level: RuntimeLogLevel::Warning,
                message: message.into(),
                source: None,
                notification: DiagnosticNotification::default(),
            }),
            None,
        )?;
        match continuation {
            CandidateSaveContinuation::Autosave => self.finish_builtin_autosave(false),
            CandidateSaveContinuation::SystemMenu { .. } => {
                self.system_menu = SystemMenuState::SaveSlots;
                self.render_slot_menu(true)
            }
        }
    }

    pub(in super::super) fn commit_candidate_save(
        &mut self,
        continuation: CandidateSaveContinuation,
    ) -> Result<(), RuntimeError> {
        let candidate = self.pending_candidate_commit.take().ok_or_else(|| {
            RuntimeError::Internal("candidate storage completion has no prepared state".into())
        })?;
        self.vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("candidate commit has no VM".into()))?
            .commit_candidate_state(candidate.state)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.presentation = candidate.presentation;
        self.pending_presentation_update = false;
        self.project_snapshot = candidate.project_snapshot;
        self.message_skip = candidate.message_skip;
        self.skip_print = candidate.skip_print;
        self.user_defined_skip = candidate.user_defined_skip;
        self.saved_skip = candidate.saved_skip;
        self.force_kana_mode = candidate.force_kana_mode;
        self.emit_presentation()?;
        for effect in candidate.effects {
            self.emit_effect(effect)?;
        }
        match continuation {
            CandidateSaveContinuation::Autosave => self.finish_builtin_autosave(true),
            CandidateSaveContinuation::SystemMenu { request, .. } => {
                if let Some(slot) = candidate.save_slot {
                    let random = self
                        .vm
                        .as_ref()
                        .ok_or_else(|| RuntimeError::Internal("candidate commit has no VM".into()))?
                        .export_random_state()
                        .map_err(|error| RuntimeError::Internal(error.to_string()))?;
                    self.establish_input_undo_checkpoint(slot, candidate.save_bytes, random)?;
                }
                self.system_menu_host_request = None;
                self.system_menu = SystemMenuState::Title;
                self.load_slot_paths.clear();
                self.occupied_slot_paths.clear();
                self.resume_storage_host(request, Vec::new())
            }
        }
    }
}
