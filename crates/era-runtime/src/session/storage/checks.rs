//! Header-only CHKDATA/CHKCHARADATA reads; checking a slot must not restore its payload.
#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in super::super) fn complete_save_check(
        &mut self,
        message_id: u64,
        pending: PendingStorage,
        result: StorageResult,
    ) -> Result<(), RuntimeError> {
        let PendingStorage::HostCheck {
            request,
            kind,
            path,
            mut data,
            change_token: expected_token,
        } = pending
        else {
            return Err(RuntimeError::Internal(
                "save check has no pending read".into(),
            ));
        };
        let (chunk, offset, complete, change_token) = match result {
            StorageResult::Error { error } => {
                let (status, description) = if error.kind == FrontendIoErrorKind::NotFound {
                    (1, "----")
                } else {
                    (4, error.message.as_str())
                };
                return self.finish_save_check(request, kind, status, description, 0);
            }
            StorageResult::ReadChunk {
                data,
                offset,
                complete,
                change_token,
            } => (data, offset, complete, change_token),
            _ => {
                self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "save check response kind differs from its request",
                )?;
                return self.finish_save_check(
                    request,
                    kind,
                    4,
                    "invalid save check response kind",
                    0,
                );
            }
        };
        if offset != data.len() as u64
            || expected_token
                .as_ref()
                .is_some_and(|expected| expected != &change_token)
        {
            self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "save check metadata chunks are not contiguous",
            )?;
            return self.finish_save_check(
                request,
                kind,
                4,
                "save check metadata changed during reading",
                0,
            );
        }
        let limits = era_runtime_save::SaveCodecLimits::default();
        let maximum_chunk = (super::SAVE_CHECK_CHUNK_BYTES as usize)
            .min(limits.maximum_bytes.saturating_sub(data.len()));
        if chunk.as_slice().len() > maximum_chunk || (!complete && chunk.as_slice().is_empty()) {
            return self.finish_save_check(request, kind, 4, "invalid save metadata chunk size", 0);
        }
        data.extend_from_slice(chunk.as_slice());
        let vm = self
            .vm
            .as_ref()
            .ok_or_else(|| RuntimeError::Internal("save check completion has no VM".into()))?;
        if kind == era_runtime_save::SaveFileKind::Normal
            && vm.vm().artifact().manifest.compatibility.profile
                == erabasic_compat::CompatibilityProfileId::EmueraEm
            && let Ok(era_runtime_save::SaveMetadataInspection::Complete {
                kind: actual_kind,
                metadata,
                ..
            }) = era_runtime_save::inspect_identity(&data, complete, limits)
        {
            let compatibility = vm
                .vm()
                .artifact()
                .project_data
                .save_load_context()
                .compatibility;
            if actual_kind == kind
                && compatibility.unique_code_matches(metadata.unique_code)
                && !compatibility.version_matches(metadata.version)
            {
                return self.finish_save_check(request, kind, 3, "", metadata.version);
            }
        }
        match era_runtime_save::inspect_metadata(&data, complete, limits) {
            Ok(era_runtime_save::SaveMetadataInspection::NeedMore) => {
                if data.len() >= limits.maximum_bytes {
                    return self.finish_save_check(
                        request,
                        kind,
                        4,
                        "save metadata exceeds limit",
                        0,
                    );
                }
                let offset = data.len() as u64;
                let maximum_bytes = u32::try_from(
                    (super::SAVE_CHECK_CHUNK_BYTES as usize)
                        .min(limits.maximum_bytes.saturating_sub(data.len())),
                )
                .unwrap_or(u32::MAX);
                self.issue_storage(
                    PendingStorage::HostCheck {
                        request,
                        kind,
                        path: path.clone(),
                        data,
                        change_token: Some(change_token.clone()),
                    },
                    if kind == era_runtime_save::SaveFileKind::Normal {
                        StorageNamespace::Save
                    } else {
                        StorageNamespace::Data
                    },
                    StorageOperation::ReadRange {
                        offset,
                        maximum_bytes,
                        change_token: Some(change_token),
                    },
                    path,
                )
            }
            Ok(era_runtime_save::SaveMetadataInspection::Complete {
                kind: actual_kind,
                metadata,
                ..
            }) => {
                let vm = self.vm.as_ref().ok_or_else(|| {
                    RuntimeError::Internal("save check completion has no VM".into())
                })?;
                let project = &vm.vm().artifact().project_data;
                let (status, description) = if actual_kind != kind {
                    (4, "different save kind")
                } else if metadata.unique_code != project.static_data.game_base.unique_code {
                    (2, "")
                } else if !project
                    .save_load_context()
                    .compatibility
                    .accepts(metadata.unique_code, metadata.version)
                {
                    (3, "")
                } else {
                    (0, metadata.description.as_str())
                };
                let version = if matches!(status, 0 | 3) {
                    metadata.version
                } else {
                    0
                };
                self.finish_save_check(request, kind, status, description, version)
            }
            Err(error) => self.finish_save_check(request, kind, 4, &error.to_string(), 0),
        }
    }

    fn finish_save_check(
        &mut self,
        request: erabasic_vm::HostRequestId,
        kind: era_runtime_save::SaveFileKind,
        status: i64,
        description: &str,
        version: i64,
    ) -> Result<(), RuntimeError> {
        let writes = self.check_data_writes(description, kind, version)?;
        self.resume_storage_host_value(request, VmValue::Integer(status), writes)
    }
}
