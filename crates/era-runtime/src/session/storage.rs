//! Transactional save, load, and frontend-provided storage orchestration.

// This is one part of the same split `RuntimeSession` implementation.
#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(super) fn begin_candidate_save(
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
                            severity: DiagnosticSeverity::Warning,
                            message:
                                "frontend storage cannot provide revision-checked atomic writes"
                                    .into(),
                            source: None,
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

    pub(super) fn begin_system_menu_candidate(&mut self, slot: u32) -> Result<(), RuntimeError> {
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

    pub(super) fn issue_candidate_clock(
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
    pub(super) fn prepare_candidate_save(
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
        let original_effect_journal = std::mem::take(&mut self.effect_journal);
        let original_sequence = self.outbound_sequence;
        let original_message = self.next_message_id;
        let original_effect = self.next_effect_id;
        self.candidate_clock = Some(time);

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
        self.effect_journal = original_effect_journal;
        self.outbound_sequence = original_sequence;
        self.next_message_id = original_message;
        self.next_effect_id = original_effect;
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

    pub(super) fn finish_candidate_save_failure(
        &mut self,
        continuation: CandidateSaveContinuation,
        message: &str,
    ) -> Result<(), RuntimeError> {
        self.pending_candidate_commit = None;
        self.emit(
            RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                code: "runtime.candidate_save_failed".into(),
                severity: DiagnosticSeverity::Warning,
                message: message.into(),
                source: None,
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

    pub(super) fn commit_candidate_save(
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

    pub(super) fn issue_storage(
        &mut self,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        let request_id = self.allocate_request()?;
        self.operations.insert_storage(request_id, pending);
        self.set_phase(RuntimePhase::WaitingExternal)?;
        self.emit(
            RuntimeMessage::StorageRequest(StorageRequest {
                request_id,
                namespace,
                relative_path,
                operation,
                idempotency_key: format!(
                    "{}-{}-{}",
                    self.options.session_id.low, self.epoch.0, request_id
                ),
                deadline_ns: None,
            }),
            None,
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn complete_storage(
        &mut self,
        message_id: u64,
        response: StorageResponse,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.operations.take_storage(response.request_id) else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "storage response has no pending request",
            );
        };
        match (pending, response.result) {
            (
                PendingStorage::KeyMacroWrite { resume_phase }
                | PendingStorage::SystemOutputLog { resume_phase },
                StorageResult::Written { .. },
            ) => self.set_phase(resume_phase),
            (PendingStorage::KeyMacroWrite { resume_phase }, StorageResult::Error { error }) => {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.key_macro_persistence_failed".into(),
                        severity: DiagnosticSeverity::Warning,
                        message: format!("macro.txt write failed: {error:?}"),
                        source: None,
                    }),
                    Some(message_id),
                )?;
                self.set_phase(resume_phase)
            }
            (PendingStorage::SystemOutputLog { resume_phase }, StorageResult::Error { error }) => {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        code: "runtime.system_output_failed".into(),
                        severity: DiagnosticSeverity::Warning,
                        message: format!("emuera.log write failed: {error:?}"),
                        source: None,
                    }),
                    Some(message_id),
                )?;
                self.set_phase(resume_phase)
            }
            (
                PendingStorage::CandidateSaveStat { slot, continuation },
                StorageResult::Metadata(metadata),
            ) => {
                let Some(revision) = metadata.revision else {
                    return self.finish_candidate_save_failure(
                        continuation,
                        "frontend stat omitted the revision required for an overwrite",
                    );
                };
                self.issue_candidate_clock(
                    slot,
                    StoragePrecondition::Revision(revision),
                    continuation,
                )
            }
            (
                PendingStorage::CandidateSaveStat { slot, continuation },
                StorageResult::Error { error },
            ) if error.kind == FrontendIoErrorKind::NotFound => {
                self.issue_candidate_clock(slot, StoragePrecondition::Missing, continuation)
            }
            (
                PendingStorage::CandidateSaveStat { continuation, .. },
                StorageResult::Error { error },
            ) => self.finish_candidate_save_failure(
                continuation,
                &format!("candidate stat failed: {error:?}"),
            ),
            (
                PendingStorage::CandidateSaveWrite { continuation },
                StorageResult::Written { .. },
            ) => self.commit_candidate_save(continuation),
            (
                PendingStorage::CandidateSaveWrite { continuation },
                StorageResult::Error { error },
            ) => self.finish_candidate_save_failure(
                continuation,
                &format!("candidate write failed: {error:?}"),
            ),
            (PendingStorage::HostFunctionWrite { request }, StorageResult::Written { .. })
            | (PendingStorage::HostStat { request }, StorageResult::Metadata(_))
            | (PendingStorage::GraphicsImageWrite { request }, StorageResult::Written { .. }) => {
                self.resume_storage_host_value(request, VmValue::Integer(1), Vec::new())
            }
            (PendingStorage::HostFunctionWrite { request }, StorageResult::Error { .. })
            | (PendingStorage::HostStat { request }, StorageResult::Error { .. })
            | (PendingStorage::GraphicsImageRead { request, .. }, StorageResult::Error { .. })
            | (PendingStorage::GraphicsImageWrite { request }, StorageResult::Error { .. }) => {
                self.resume_storage_host_value(request, VmValue::Integer(0), Vec::new())
            }
            (PendingStorage::HostReadText { request }, StorageResult::Read { data, .. }) => {
                let text = std::str::from_utf8(data.as_slice())
                    .map(|value| value.trim_start_matches('\u{feff}').replace('\r', ""))
                    .unwrap_or_default();
                self.resume_storage_host_value(request, VmValue::String(text), Vec::new())
            }
            (PendingStorage::HostReadText { request }, StorageResult::Error { .. }) => {
                self.resume_storage_host_value(request, VmValue::String(String::new()), Vec::new())
            }
            (
                PendingStorage::GraphicsImageRead { request, canvas_id },
                StorageResult::Read { data, .. },
            ) => {
                if self.service_capabilities.get(&(
                    ServiceKind::Canvas,
                    DECODE_CANVAS_IMAGE_OPERATION.to_owned(),
                )) != Some(&DECODE_CANVAS_IMAGE_OPERATION_VERSION)
                {
                    return self.resume_storage_host_value(
                        request,
                        VmValue::Integer(0),
                        Vec::new(),
                    );
                }
                let encoded = data.as_slice().to_vec();
                let request_id = self.allocate_request()?;
                self.operations.insert_service(
                    request_id,
                    PendingService::Host(ExternalCompletion::DecodeCanvasImage {
                        request,
                        canvas_id,
                        encoded: encoded.clone(),
                    }),
                );
                self.emit(
                    RuntimeMessage::ServiceRequest(ServiceRequest {
                        request_id,
                        kind: ServiceKind::Canvas,
                        operation: DECODE_CANVAS_IMAGE_OPERATION.into(),
                        operation_version: DECODE_CANVAS_IMAGE_OPERATION_VERSION,
                        payload: ProtocolBytes::new(encode_canonical(&DecodeCanvasImageRequest {
                            encoded: ProtocolBytes::new(encoded),
                        })?),
                        deadline_ns: None,
                    }),
                    None,
                )
            }
            (
                PendingStorage::HostListFiles {
                    request,
                    target,
                    strip_character_dat,
                },
                StorageResult::Listed { mut entries },
            ) => {
                entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
                let values = entries
                    .iter()
                    .map(|entry| {
                        if strip_character_dat {
                            entry
                                .relative_path
                                .strip_prefix("chara_")
                                .and_then(|value| value.strip_suffix(".dat"))
                                .unwrap_or(&entry.relative_path)
                                .to_owned()
                        } else {
                            entry.relative_path.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                let writes = self.file_list_writes(target, &values)?;
                self.resume_storage_host_value(
                    request,
                    VmValue::Integer(i64::try_from(entries.len()).unwrap_or(i64::MAX)),
                    writes,
                )
            }
            (PendingStorage::HostListFiles { request, .. }, StorageResult::Error { .. }) => {
                self.resume_storage_host_value(request, VmValue::Integer(-1), Vec::new())
            }
            (PendingStorage::HostWrite { request }, StorageResult::Written { .. })
            | (PendingStorage::HostDelete { request }, StorageResult::Deleted) => {
                self.resume_storage_host(request, Vec::new())
            }
            (PendingStorage::HostDelete { request }, StorageResult::Error { error })
                if error.kind == FrontendIoErrorKind::NotFound =>
            {
                // DELDATA is explicitly idempotent in the reference runtime.
                self.resume_storage_host(request, Vec::new())
            }
            (PendingStorage::HostLoadGlobal { request }, StorageResult::Error { error })
                if error.kind == FrontendIoErrorKind::NotFound =>
            {
                let writes = self.result_write(0)?;
                self.resume_storage_host(request, writes)
            }
            (PendingStorage::HostLoadGlobal { request }, StorageResult::Read { data, .. }) => {
                self.complete_global_load(request, data.as_slice())
            }
            (PendingStorage::HostLoadCharacters { request }, StorageResult::Read { data, .. }) => {
                self.complete_character_load(request, data.as_slice())
            }
            (PendingStorage::HostLoadCharacters { request }, StorageResult::Error { .. }) => {
                let writes = self.result_write(0)?;
                self.resume_storage_host(request, writes)
            }
            (PendingStorage::HostCheck { request, .. }, StorageResult::Error { error }) => {
                let status = if error.kind == FrontendIoErrorKind::NotFound {
                    1
                } else {
                    4
                };
                let writes = self.check_data_writes(&error.message)?;
                self.resume_storage_host_value(request, VmValue::Integer(status), writes)
            }
            (PendingStorage::HostCheck { request, kind }, StorageResult::Read { data, .. }) => {
                let vm = self.vm.as_ref().ok_or_else(|| {
                    RuntimeError::Internal("save check completion has no VM".into())
                })?;
                let (status, description) =
                    match decode_scoped_save(data.as_slice(), vm.vm().artifact(), kind) {
                        Ok(decoded) => {
                            let game_base = &vm.vm().artifact().project_data.static_data.game_base;
                            if decoded.state.unique_code != game_base.unique_code {
                                (2, String::new())
                            } else if !vm
                                .vm()
                                .artifact()
                                .project_data
                                .save_load_context()
                                .compatibility
                                .accepts(decoded.state.unique_code, decoded.state.version)
                            {
                                (3, String::new())
                            } else {
                                (0, decoded.description)
                            }
                        }
                        Err(error) => (4, error.to_string()),
                    };
                let writes = self.check_data_writes(&description)?;
                self.resume_storage_host_value(request, VmValue::Integer(status), writes)
            }
            (PendingStorage::HostLoadOrdinary { slot }, StorageResult::Read { data, .. }) => {
                self.complete_ordinary_load(slot, data.as_slice())
            }
            (PendingStorage::ListLoadSlots, StorageResult::Listed { entries }) => {
                self.open_slot_menu(message_id, entries, false)
            }
            (PendingStorage::ListSaveSlots, StorageResult::Listed { entries }) => {
                self.open_slot_menu(message_id, entries, true)
            }
            (
                PendingStorage::ScanMenuSlot {
                    save,
                    path,
                    remaining,
                    mut data,
                    change_token: expected_token,
                },
                StorageResult::ReadChunk {
                    data: chunk,
                    offset,
                    complete,
                    change_token,
                },
            ) => {
                if offset != data.len() as u64
                    || expected_token
                        .as_ref()
                        .is_some_and(|expected| expected != &change_token)
                {
                    return self.reject(
                        message_id,
                        CommandErrorCode::StaleRequest,
                        "save metadata chunks are not contiguous",
                    );
                }
                data.extend_from_slice(chunk.as_slice());
                let inspection = era_runtime_save::inspect_metadata(
                    &data,
                    complete,
                    era_runtime_save::SaveCodecLimits::default(),
                );
                if matches!(
                    inspection,
                    Ok(era_runtime_save::SaveMetadataInspection::NeedMore)
                ) {
                    let maximum = era_runtime_save::SaveCodecLimits::default().maximum_bytes;
                    if data.len() >= maximum {
                        self.invalid_slot_paths.insert(path.clone());
                        self.slot_labels
                            .insert(path, "corrupt: metadata exceeds limit".into());
                        return self.scan_next_menu_slot(save, remaining);
                    }
                    let next = (64 * 1024usize).min(maximum.saturating_sub(data.len()));
                    let next_offset = data.len() as u64;
                    return self.issue_storage(
                        PendingStorage::ScanMenuSlot {
                            save,
                            path: path.clone(),
                            remaining,
                            data,
                            change_token: Some(change_token.clone()),
                        },
                        StorageNamespace::Save,
                        StorageOperation::ReadRange {
                            offset: next_offset,
                            maximum_bytes: u32::try_from(next).unwrap_or(u32::MAX),
                            change_token: Some(change_token),
                        },
                        path,
                    );
                }
                let vm = self
                    .vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("save menu scan has no VM".into()))?;
                let status = match inspection {
                    Ok(era_runtime_save::SaveMetadataInspection::Complete {
                        kind: era_runtime_save::SaveFileKind::Normal,
                        metadata,
                        ..
                    }) => {
                        let game = &vm.vm().artifact().project_data.static_data.game_base;
                        if metadata.unique_code != game.unique_code {
                            Err("different game".to_owned())
                        } else if !vm
                            .vm()
                            .artifact()
                            .project_data
                            .save_load_context()
                            .compatibility
                            .accepts(metadata.unique_code, metadata.version)
                        {
                            Err("different version".to_owned())
                        } else {
                            Ok(metadata.description)
                        }
                    }
                    Ok(era_runtime_save::SaveMetadataInspection::Complete { .. }) => {
                        Err("different save kind".to_owned())
                    }
                    Ok(era_runtime_save::SaveMetadataInspection::NeedMore) => unreachable!(),
                    Err(error) => Err(format!("corrupt: {error}")),
                };
                self.slot_change_tokens.insert(path.clone(), change_token);
                match status {
                    Ok(label) => {
                        self.slot_labels.insert(path, label);
                    }
                    Err(label) => {
                        self.invalid_slot_paths.insert(path.clone());
                        self.slot_labels.insert(path, label);
                    }
                }
                self.scan_next_menu_slot(save, remaining)
            }
            (
                PendingStorage::ScanMenuSlot {
                    save,
                    path,
                    remaining,
                    ..
                },
                StorageResult::Error { error },
            ) => {
                if error.kind == FrontendIoErrorKind::NotFound {
                    self.occupied_slot_paths.remove(&path);
                    self.slot_change_tokens.remove(&path);
                } else {
                    self.invalid_slot_paths.insert(path.clone());
                    self.slot_labels
                        .insert(path, format!("I/O error: {}", error.message));
                }
                self.scan_next_menu_slot(save, remaining)
            }
            (
                PendingStorage::StatDeleteMenuSlot { save, path },
                StorageResult::Metadata(metadata),
            ) => {
                let Some(revision) = metadata.revision else {
                    self.system_menu = if save {
                        SystemMenuState::SaveSlots
                    } else {
                        SystemMenuState::LoadSlots
                    };
                    return self.render_slot_menu(save);
                };
                self.issue_storage(
                    PendingStorage::DeleteMenuSlot {
                        save,
                        path: path.clone(),
                    },
                    StorageNamespace::Save,
                    StorageOperation::Delete {
                        precondition: StoragePrecondition::Revision(revision),
                    },
                    path,
                )
            }
            (
                PendingStorage::StatDeleteMenuSlot { save, path }
                | PendingStorage::DeleteMenuSlot { save, path },
                StorageResult::Error { error },
            ) if error.kind == FrontendIoErrorKind::NotFound => {
                self.occupied_slot_paths.remove(&path);
                self.slot_change_tokens.remove(&path);
                self.slot_labels.remove(&path);
                self.render_slot_menu(save)
            }
            (PendingStorage::DeleteMenuSlot { save, path }, StorageResult::Deleted) => {
                self.occupied_slot_paths.remove(&path);
                self.slot_change_tokens.remove(&path);
                self.slot_labels.remove(&path);
                self.system_menu = if save {
                    SystemMenuState::SaveSlots
                } else {
                    SystemMenuState::LoadSlots
                };
                self.render_slot_menu(save)
            }
            (
                PendingStorage::StatDeleteMenuSlot { save, .. }
                | PendingStorage::DeleteMenuSlot { save, .. },
                StorageResult::Error { error },
            ) => {
                self.presentation.append_system_text(
                    format!("delete failed: {error:?}"),
                    SystemTextKey::InvalidValue,
                    Vec::new(),
                    true,
                );
                self.render_slot_menu(save)
            }
            (PendingStorage::ReadLoadSlot { slot }, StorageResult::Read { data, .. }) => {
                let vm = self.vm.as_ref().ok_or_else(|| {
                    RuntimeError::Internal("system load completion has no VM".into())
                })?;
                let valid = decode_scoped_save(
                    data.as_slice(),
                    vm.vm().artifact(),
                    era_runtime_save::SaveFileKind::Normal,
                )
                .ok()
                .is_some_and(|decoded| {
                    let game = &vm.vm().artifact().project_data.static_data.game_base;
                    decoded.state.unique_code == game.unique_code
                        && vm
                            .vm()
                            .artifact()
                            .project_data
                            .save_load_context()
                            .compatibility
                            .accepts(decoded.state.unique_code, decoded.state.version)
                });
                if !valid {
                    self.presentation.append_system_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        SystemTextKey::InvalidValue,
                        Vec::new(),
                        true,
                    );
                    return self.render_slot_menu(false);
                }
                self.system_menu_host_request = None;
                self.complete_ordinary_load(slot, data.as_slice())
            }
            (pending, StorageResult::Error { error }) => {
                if matches!(
                    pending,
                    PendingStorage::HostWrite { .. }
                        | PendingStorage::HostDelete { .. }
                        | PendingStorage::HostLoadOrdinary { .. }
                        | PendingStorage::HostLoadGlobal { .. }
                        | PendingStorage::HostLoadCharacters { .. }
                        | PendingStorage::HostCheck { .. }
                        | PendingStorage::HostFunctionWrite { .. }
                        | PendingStorage::HostReadText { .. }
                        | PendingStorage::HostStat { .. }
                        | PendingStorage::HostListFiles { .. }
                        | PendingStorage::GraphicsImageRead { .. }
                        | PendingStorage::GraphicsImageWrite { .. }
                ) {
                    return self.fault(
                        FaultCode::ServiceFailure,
                        &format!("storage operation failed: {error:?}"),
                        None,
                    );
                }
                self.presentation.append_system_text(
                    format!(
                        "{}: {error:?}",
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue)
                    ),
                    SystemTextKey::InvalidValue,
                    Vec::new(),
                    true,
                );
                if matches!(
                    pending,
                    PendingStorage::ListLoadSlots
                        | PendingStorage::ListSaveSlots
                        | PendingStorage::ReadLoadSlot { .. }
                ) && self.system_menu_host_request.is_some()
                {
                    self.resume_system_menu_host()
                } else {
                    self.open_title_menu()
                }
            }
            _ => self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "storage response kind differs from its request",
            ),
        }
    }

    pub(super) fn resume_storage_host(
        &mut self,
        request: erabasic_vm::HostRequestId,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    pub(super) fn open_slot_menu(
        &mut self,
        message_id: u64,
        mut entries: Vec<StorageEntry>,
        save: bool,
    ) -> Result<(), RuntimeError> {
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if entries.iter().any(|entry| {
            era_runtime_protocol::validate_relative_path(&entry.relative_path).is_err()
        }) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "storage list contains an invalid relative path",
            );
        }
        let previous_tokens = std::mem::take(&mut self.slot_change_tokens);
        self.occupied_slot_paths = entries
            .iter()
            .map(|entry| entry.relative_path.clone())
            .collect();
        self.slot_change_tokens = entries
            .into_iter()
            .filter_map(|entry| entry.change_token.map(|token| (entry.relative_path, token)))
            .collect();
        self.slot_labels
            .retain(|path, _| previous_tokens.get(path) == self.slot_change_tokens.get(path));
        self.invalid_slot_paths
            .retain(|path| previous_tokens.get(path) == self.slot_change_tokens.get(path));
        self.system_menu = if save {
            SystemMenuState::SaveSlots
        } else {
            SystemMenuState::LoadSlots
        };
        self.scan_slot_page(save)
    }

    pub(super) fn scan_slot_page(&mut self, save: bool) -> Result<(), RuntimeError> {
        let mut remaining = self.slot_page_paths(save);
        remaining.retain(|path| {
            self.occupied_slot_paths.contains(path) && !self.slot_labels.contains_key(path)
        });
        remaining.reverse();
        self.scan_next_menu_slot(save, remaining)
    }

    pub(super) fn scan_next_menu_slot(
        &mut self,
        save: bool,
        mut remaining: Vec<String>,
    ) -> Result<(), RuntimeError> {
        let Some(path) = remaining.pop() else {
            return self.render_slot_menu(save);
        };
        self.issue_storage(
            PendingStorage::ScanMenuSlot {
                save,
                path: path.clone(),
                remaining,
                data: Vec::new(),
                change_token: self.slot_change_tokens.get(&path).cloned(),
            },
            StorageNamespace::Save,
            StorageOperation::ReadRange {
                offset: 0,
                maximum_bytes: 64 * 1024,
                change_token: self.slot_change_tokens.get(&path).cloned(),
            },
            path,
        )
    }

    fn slot_page_paths(&mut self, save: bool) -> Vec<String> {
        let slot_count = self
            .project_snapshot
            .as_ref()
            .map_or(20, |snapshot| snapshot.save_slot_count)
            .max(20);
        let page_count = slot_count.div_ceil(20);
        self.system_menu_page = self.system_menu_page.min(page_count.saturating_sub(1));
        let start = self.system_menu_page.saturating_mul(20);
        let end = start.saturating_add(20).min(slot_count);
        let mut paths = (start..end).map(save_slot_path).collect::<Vec<_>>();
        if !save {
            paths.push(save_slot_path(99));
        }
        paths
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn render_slot_menu(&mut self, save: bool) -> Result<(), RuntimeError> {
        let slot_count = self
            .project_snapshot
            .as_ref()
            .map_or(20, |snapshot| snapshot.save_slot_count)
            .max(20);
        let page_count = slot_count.div_ceil(20);
        self.system_menu_page = self.system_menu_page.min(page_count.saturating_sub(1));
        self.load_slot_paths = self.slot_page_paths(save);
        let question = if save {
            SystemTextKey::SaveQuestion
        } else {
            SystemTextKey::LoadQuestion
        };
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, question),
            question,
            Vec::new(),
            false,
        );
        let mut choices = BTreeMap::new();
        for index in 0..self.load_slot_paths.len() {
            let path = self.load_slot_paths[index].clone();
            let slot = parse_save_slot(&path).unwrap_or(u32::MAX);
            let occupied = self.occupied_slot_paths.contains(&path);
            let token = self.allocate_interaction();
            let label = if occupied {
                format!(
                    "[{slot:>2}] {}",
                    self.slot_labels
                        .get(&path)
                        .map_or("(unreadable)", String::as_str)
                )
            } else {
                format!("[{slot:>2}] ----")
            };
            self.presentation.append_system_button(
                label,
                SystemTextKey::SaveSlot,
                vec![SystemTextArgument::String(path.clone())],
                token,
            );
            choices.insert(token, VmValue::Integer(i64::from(slot)));
            if occupied && self.storage_capabilities.delete && self.storage_capabilities.revisions {
                let delete = self.allocate_interaction();
                self.presentation.append_system_button(
                    format!("Delete {}", self.load_slot_paths[index]),
                    SystemTextKey::SaveSlot,
                    vec![SystemTextArgument::String(
                        self.load_slot_paths[index].clone(),
                    )],
                    delete,
                );
                choices.insert(
                    delete,
                    VmValue::Integer(-1_000 - i64::try_from(index).unwrap_or(i64::MAX)),
                );
            }
        }
        let back = self.allocate_interaction();
        self.presentation.append_system_button(
            localized_system_text(&self.selected_locale, SystemTextKey::Back),
            SystemTextKey::Back,
            Vec::new(),
            back,
        );
        choices.insert(back, VmValue::Integer(100));
        for page in self.system_menu_page.saturating_add(1)..page_count {
            let first = page.saturating_mul(20);
            let last = first.saturating_add(19).min(slot_count.saturating_sub(1));
            let token = self.allocate_interaction();
            self.presentation.append_system_button(
                format!("[{first}-{last}]"),
                SystemTextKey::SaveSlot,
                vec![SystemTextArgument::Integer(i64::from(first))],
                token,
            );
            choices.insert(token, VmValue::Integer(i64::from(first)));
        }
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        wait.kind = WaitKind::IntegerValue;
        self.open_wait(
            PendingInput {
                host_request: self.system_menu_host_request,
                wait,
                result_name: None,
                choices,
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )
    }

    pub(super) fn resume_system_menu_host(&mut self) -> Result<(), RuntimeError> {
        let Some(request) = self.system_menu_host_request.take() else {
            return self.open_title_menu();
        };
        self.system_menu = SystemMenuState::Title;
        self.load_slot_paths.clear();
        self.occupied_slot_paths.clear();
        self.slot_change_tokens.clear();
        self.slot_labels.clear();
        self.resume_storage_host(request, Vec::new())
    }

    pub(super) fn finish_builtin_autosave(&mut self, success: bool) -> Result<(), RuntimeError> {
        if !success {
            return self.stage_builtin_autosave_failure();
        }
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("autosave completion has no VM".into()))?;
        if let Some(flow) = self.controller.deferred_flow.take() {
            self.controller.flow = Some(flow);
            self.begin_flow(&mut vm, flow)?;
            self.vm = Some(vm);
            self.set_phase(RuntimePhase::Running)?;
            return self.renew_debug_grant();
        }
        self.controller.step = SystemStep::ShopShow;
        self.dispatch_system_function(&mut vm, "SHOW_SHOP", true)?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        self.renew_debug_grant()
    }

    pub(super) fn stage_builtin_autosave_failure(&mut self) -> Result<(), RuntimeError> {
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, SystemTextKey::AutoSaveFailed),
            SystemTextKey::AutoSaveFailed,
            Vec::new(),
            false,
        );
        self.presentation.append_system_text(
            localized_system_text(&self.selected_locale, SystemTextKey::AutoSaveSkipped),
            SystemTextKey::AutoSaveSkipped,
            Vec::new(),
            false,
        );
        self.controller.step = SystemStep::ShopAutosaveFailureWait;
        let submission = self.allocate_interaction();
        let mut wait = self.system_wait(submission);
        wait.kind = WaitKind::EnterKey;
        wait.mouse_input = false;
        wait.default_value = None;
        self.open_wait(
            PendingInput {
                host_request: None,
                wait,
                result_name: None,
                choices: BTreeMap::new(),
                timeout_duration_ns: None,
                post_input: None,
            },
            true,
        )?;
        self.renew_debug_grant()
    }

    pub(super) fn resume_storage_host_value(
        &mut self,
        request: erabasic_vm::HostRequestId,
        value: VmValue,
        writes: Vec<HostWrite>,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: Some(value),
                writes,
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    pub(super) fn file_list_writes(
        &self,
        target: Option<PlaceDescriptor>,
        values: &[String],
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("file list completion has no VM".into()))?;
        let base = target.or_else(|| global_place_at(vm, "RESULTS", 0));
        let Some(base) = base else {
            return Ok(Vec::new());
        };
        let maximum = vm
            .vm()
            .artifact()
            .globals
            .iter()
            .find(|definition| definition.key == base.variable)
            .and_then(|definition| definition.dimensions.first())
            .and_then(|value| usize::try_from(*value).ok())
            .unwrap_or(0);
        Ok(values
            .iter()
            .take(maximum)
            .enumerate()
            .map(|(index, value)| {
                let mut target = base.clone();
                if let Some(last) = target.indices.last_mut() {
                    *last = u64::try_from(index).unwrap_or(u64::MAX);
                } else {
                    target
                        .indices
                        .push(u64::try_from(index).unwrap_or(u64::MAX));
                }
                HostWrite {
                    target,
                    value: VmValue::String(value.clone()),
                }
            })
            .collect())
    }

    pub(super) fn result_write(&self, value: i64) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("storage completion has no VM".into()))?;
        Ok(global_place(vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(value),
                }]
            })
            .unwrap_or_default())
    }

    pub(super) fn check_data_writes(
        &self,
        description: &str,
    ) -> Result<Vec<HostWrite>, RuntimeError> {
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("save check completion has no VM".into()))?;
        let mut writes = Vec::new();
        if let Some(target) = global_place(vm, "RESULTS") {
            writes.push(HostWrite {
                target,
                value: VmValue::String(description.to_owned()),
            });
        }
        Ok(writes)
    }

    pub(super) fn complete_global_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("global load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Global,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid global save: {error}")))?;
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::OverlayGlobal(Box::new(decoded.state)),
                StructuredScope::Global,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions =
            merge_opaque_extensions(&self.save_extensions, decoded.opaque_extensions);
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }

    pub(super) fn complete_ordinary_load(
        &mut self,
        slot: u32,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let establish_undo = self.undo_replay.is_none();
        let random_before_load = establish_undo
            .then(|| {
                self.vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?
                    .export_random_state()
                    .map_err(|error| RuntimeError::Internal(error.to_string()))
            })
            .transpose()?;
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("ordinary load has no VM".into()))?;
        let decoded = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
        )
        .map_err(|error| RuntimeError::Internal(format!("invalid ordinary save: {error}")))?;
        let version = decoded.state.version;
        let description = decoded.description.clone();
        let (prepared, _) = vm
            .prepare_runtime_state_with_extensions(
                VmRuntimeStateTransaction::RestoreOrdinary(Box::new(decoded.state)),
                StructuredScope::Ordinary,
                &decoded.structured_extensions,
            )
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let last_load = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::SetLastLoad {
                version,
                slot: i64::from(slot),
                text: description,
            })
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(last_load)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        self.save_extensions = decoded.opaque_extensions;
        self.advance_epoch();
        self.controller.clear();
        self.controller.flow = Some(SystemFlow::Shop);
        self.controller.step = SystemStep::PostLoadShop;
        self.controller.prepare_load_sequence(vm.vm().artifact());
        if self.controller.is_complete() {
            self.continue_system_flow(&mut vm)?;
        } else {
            self.spawn_next_event(&mut vm)?;
        }
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)?;
        if let Some(random) = random_before_load {
            self.establish_input_undo_checkpoint(slot, bytes.to_vec(), random)?;
        }
        Ok(())
    }

    pub(super) fn complete_character_load(
        &mut self,
        request: erabasic_vm::HostRequestId,
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        let mut vm = self
            .vm
            .take()
            .ok_or_else(|| RuntimeError::Internal("character load has no VM".into()))?;
        let Ok(decoded) = decode_scoped_save(
            bytes,
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Character,
        ) else {
            let writes = global_place(&vm, "RESULT")
                .map(|target| {
                    vec![HostWrite {
                        target,
                        value: VmValue::Integer(0),
                    }]
                })
                .unwrap_or_default();
            commit_completion(
                &mut vm,
                request,
                VmHostCompletion::Ready(HostReady {
                    value: None,
                    writes,
                }),
            )?;
            self.vm = Some(vm);
            return self.set_phase(RuntimePhase::Running);
        };
        let prepared = vm
            .prepare_runtime_state(VmRuntimeStateTransaction::AppendCharacters(Box::new(
                decoded.state,
            )))
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        vm.commit_runtime_state(prepared)
            .map_err(|error| RuntimeError::Internal(error.to_string()))?;
        let writes = global_place(&vm, "RESULT")
            .map(|target| {
                vec![HostWrite {
                    target,
                    value: VmValue::Integer(1),
                }]
            })
            .unwrap_or_default();
        commit_completion(
            &mut vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: None,
                writes,
            }),
        )?;
        self.vm = Some(vm);
        self.set_phase(RuntimePhase::Running)
    }
}
