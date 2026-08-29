// This is part of the split RuntimeSession storage implementation.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    pub(in super::super) fn issue_storage(
        &mut self,
        pending: PendingStorage,
        namespace: StorageNamespace,
        operation: StorageOperation,
        relative_path: String,
    ) -> Result<(), RuntimeError> {
        let request_id = self.allocate_request()?;
        self.operations.insert_storage(request_id, pending);
        if let Err(error) = self.set_phase(RuntimePhase::WaitingExternal) {
            self.operations.take_storage(request_id);
            return Err(error);
        }
        let result = self.emit(
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
        );
        if result.is_err() {
            self.operations.take_storage(request_id);
        }
        result
    }

    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn complete_storage(
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
                pending @ (PendingStorage::SqlSeedRead { .. }
                | PendingStorage::SqlMapXmlRead { .. }),
                result,
            ) => self.complete_sql_storage(message_id, pending, result),
            (
                pending @ (PendingStorage::HostResourceText { .. }
                | PendingStorage::HostResourceStat { .. }
                | PendingStorage::HostResourceList { .. }),
                result,
            ) => self.complete_resource_storage(pending, result),
            (
                PendingStorage::KeyMacroWrite { resume_phase }
                | PendingStorage::SystemOutputLog { resume_phase },
                StorageResult::Written { .. },
            ) => self.set_phase(resume_phase),
            (PendingStorage::KeyMacroWrite { resume_phase }, StorageResult::Error { error }) => {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        context: None,
                        code: "runtime.key_macro_persistence_failed".into(),
                        level: RuntimeLogLevel::Warning,
                        message: format!("macro.txt write failed: {error:?}"),
                        source: None,
                        notification: DiagnosticNotification::default(),
                    }),
                    Some(message_id),
                )?;
                self.set_phase(resume_phase)
            }
            (PendingStorage::SystemOutputLog { resume_phase }, StorageResult::Error { error }) => {
                self.emit(
                    RuntimeMessage::Diagnostic(ProtocolDiagnostic {
                        context: None,
                        code: "runtime.system_output_failed".into(),
                        level: RuntimeLogLevel::Warning,
                        message: format!("emuera.log write failed: {error:?}"),
                        source: None,
                        notification: DiagnosticNotification::default(),
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
                let text = decode_load_text(data.as_slice()).unwrap_or_default();
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
            (PendingStorage::HostLoadGlobal { request, .. }, StorageResult::Error { error })
                if error.kind == FrontendIoErrorKind::NotFound =>
            {
                let writes = self.result_write(0)?;
                self.resume_storage_host(request, writes)
            }
            (
                PendingStorage::HostLoadGlobal {
                    request,
                    storage_path,
                },
                StorageResult::Read { data, .. },
            ) => self.complete_global_load(request, data.as_slice(), &storage_path),
            (
                PendingStorage::HostLoadCharacters {
                    request,
                    storage_path,
                },
                StorageResult::Read { data, .. },
            ) => self.complete_character_load(request, data.as_slice(), &storage_path),
            (PendingStorage::HostLoadCharacters { request, .. }, StorageResult::Error { .. }) => {
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
                let compatibility = &self
                    .vm
                    .as_ref()
                    .ok_or_else(|| RuntimeError::Internal("save menu scan has no VM".into()))?
                    .vm()
                    .artifact()
                    .manifest
                    .compatibility;
                let inspection = era_runtime_save::inspect_compatible_metadata(
                    &data,
                    complete,
                    compatibility,
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
                let Ok(decoded) = decode_scoped_save(
                    data.as_slice(),
                    vm.vm().artifact(),
                    era_runtime_save::SaveFileKind::Normal,
                ) else {
                    self.presentation.append_system_text(
                        localized_system_text(&self.selected_locale, SystemTextKey::InvalidValue),
                        SystemTextKey::InvalidValue,
                        Vec::new(),
                        true,
                    );
                    return self.render_slot_menu(false);
                };
                let valid = {
                    let game = &vm.vm().artifact().project_data.static_data.game_base;
                    decoded.state.unique_code == game.unique_code
                        && vm
                            .vm()
                            .artifact()
                            .project_data
                            .save_load_context()
                            .compatibility
                            .accepts(decoded.state.unique_code, decoded.state.version)
                };
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
                self.complete_decoded_ordinary_load(slot, data.as_slice(), decoded)
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
}
