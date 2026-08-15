#[allow(clippy::wildcard_imports)]
use super::super::*;

impl RuntimeSession {
    /// Stage an owned compiled-project cache for the next project-load request.
    ///
    /// In-process hosts use this entry point to avoid serializing an already contiguous cache
    /// through the chunked frontend protocol. The cache's embedded version, digest, identities,
    /// and bytecode are still validated by the normal project-load path before installation.
    ///
    /// # Errors
    ///
    /// Returns an error when another import is active or the cache exceeds the negotiated
    /// transfer limit.
    pub fn stage_compiled_project_cache(&mut self, bytes: Vec<u8>) -> Result<u64, RuntimeError> {
        if self.inbound_transfer.is_some() {
            return Err(RuntimeError::Busy("another state import is already active"));
        }
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            > self.options.limits.maximum_transfer_bytes
        {
            return Err(RuntimeError::ResourceLimit(
                "compiled project cache exceeds the negotiated transfer limit",
            ));
        }
        let transfer_id = self.allocate_transfer();
        self.inbound_transfer = Some(InboundStateTransfer {
            descriptor: StateTransferDescriptor {
                transfer_id,
                kind: StateExportKind::CompiledProjectCache,
                total_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                // Host staging transfers ownership in one call. The compiled-cache decoder
                // validates the format's own trailing digest before any artifact is installed.
                digest: ProtocolBytes::new(Vec::new()),
                artifact_id: None,
            },
            bytes,
            hasher: None,
            committed: true,
        });
        Ok(transfer_id)
    }

    pub(in super::super) fn stage_full_project_manifest(
        &mut self,
        message_id: u64,
        request: FullProjectManifest,
    ) -> Result<(), RuntimeError> {
        if self.full_project_task.is_some() || self.outbound_transfer.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "a project export is already active",
            );
        }
        self.full_project_failure = None;
        self.full_project_file = None;
        self.staged_full_project_manifest = Some(request.manifest);
        Ok(())
    }

    /// Return the negotiated upper bound for an in-process compiled-cache staging call.
    #[must_use]
    pub const fn maximum_transfer_bytes(&self) -> u64 {
        self.options.limits.maximum_transfer_bytes
    }
}

mod export;
mod import;
mod project_container;
