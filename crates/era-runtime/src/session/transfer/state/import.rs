#[allow(clippy::wildcard_imports)]
use super::*;

impl RuntimeSession {
    pub(in super::super::super) fn begin_state_import(
        &mut self,
        message_id: u64,
        request: StateImportBegin,
    ) -> Result<(), RuntimeError> {
        if matches!(
            request.kind,
            StateExportKind::InputReplay | StateExportKind::FullProjectFile
        ) {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "the requested state kind is export-only and cannot be imported",
            );
        }
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
        let streamed_manifest = request.kind == StateExportKind::FullProjectManifest;
        if request.digest.is_some() == streamed_manifest
            || request
                .digest
                .as_ref()
                .is_some_and(|digest| digest.as_slice().len() != blake3::OUT_LEN)
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                if streamed_manifest {
                    "full project manifest digest must be supplied at commit"
                } else {
                    "state import begin digest must contain 32 bytes"
                },
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
                digest: request
                    .digest
                    .unwrap_or_else(|| ProtocolBytes::new(Vec::new())),
                artifact_id: request.artifact_id,
            },
            // Grow with accepted chunks instead of trusting a potentially huge declaration.
            bytes: Vec::new(),
            manifest_decoder: streamed_manifest
                .then(super::manifest_import::ManifestImportDecoder::default),
            hasher: Some(blake3::Hasher::new()),
            committed: false,
        });
        self.emit(
            RuntimeMessage::StateImportAccepted(StateImportAccepted { transfer_id }),
            Some(message_id),
        )
    }

    pub(in super::super::super) fn append_state_import(
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
        if chunk.offset != transfer.received_bytes() {
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
        if let Some(decoder) = &mut transfer.manifest_decoder {
            decoder.push(chunk.data.as_slice())?;
        } else {
            transfer
                .bytes
                .try_reserve(chunk.data.as_slice().len())
                .map_err(|_| RuntimeError::ResourceLimit("state import allocation failed"))?;
            transfer.bytes.extend_from_slice(chunk.data.as_slice());
        }
        if let Some(hasher) = &mut transfer.hasher {
            hasher.update(chunk.data.as_slice());
        }
        Ok(())
    }

    pub(in super::super::super) fn commit_state_import(
        &mut self,
        message_id: u64,
        commit: &StateImportCommit,
    ) -> Result<(), RuntimeError> {
        let Some(transfer) = self.inbound_transfer.as_ref() else {
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
        let commit_digest = commit.digest.as_ref();
        let streamed_manifest = transfer.descriptor.kind == StateExportKind::FullProjectManifest;
        let expected_digest = if streamed_manifest {
            commit_digest.map(ProtocolBytes::as_slice)
        } else if commit_digest.is_none() {
            Some(transfer.descriptor.digest.as_slice())
        } else {
            None
        };
        let actual_digest = transfer
            .hasher
            .as_ref()
            .map_or_else(|| blake3::hash(&transfer.bytes), blake3::Hasher::finalize);
        if expected_digest.is_none_or(|digest| digest.len() != blake3::OUT_LEN)
            || transfer.received_bytes() != transfer.descriptor.total_bytes
            || expected_digest != Some(actual_digest.as_bytes().as_slice())
        {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidValue,
                "state import length or digest does not match its descriptor",
            );
        }
        let kind = transfer.descriptor.kind;
        if kind == StateExportKind::FullProjectManifest {
            if self.full_project_task.is_some()
                || self.outbound_transfer.is_some()
                || self.staged_full_project_manifest.is_some()
            {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidState,
                    "a project export is already active",
                );
            }
            let decoder = self
                .inbound_transfer
                .take()
                .and_then(|transfer| transfer.manifest_decoder)
                .expect("full manifest import retains its decoder");
            let Ok(manifest) = decoder.finish() else {
                return self.reject(
                    message_id,
                    CommandErrorCode::InvalidValue,
                    "full project manifest is not valid canonical CBOR",
                );
            };
            self.full_project_failure = None;
            self.full_project_file = None;
            self.staged_full_project_manifest = Some(StagedFullProjectManifest {
                source_transfer_id: Some(commit.transfer_id),
                manifest,
            });
        } else if let Some(transfer) = self.inbound_transfer.as_mut() {
            transfer.committed = true;
        }
        self.emit(
            RuntimeMessage::StateImportReady(StateImportReady {
                transfer_id: commit.transfer_id,
                kind,
            }),
            Some(message_id),
        )
    }

    pub(in super::super::super) fn read_state_export(
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
            data: ProtocolBytes::new(transfer.bytes.copy_range(offset..end)),
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

    pub(in super::super::super) fn cancel_state_transfer(
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
        let staged_full_project = self
            .staged_full_project_manifest
            .as_ref()
            .is_some_and(|staged| staged.source_transfer_id == Some(cancel.transfer_id));
        if !inbound && !outbound && !staged_full_project {
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
        if staged_full_project {
            self.staged_full_project_manifest = None;
        }
        Ok(())
    }

    pub(in super::super::super) fn consume_state_import(
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
}
