//! Exact SQL snapshot restore coordination and candidate cleanup.

#[allow(clippy::wildcard_imports)]
use super::super::super::*;
use sha2::{Digest as _, Sha256};

impl RuntimeSession {
    pub(in crate::session) fn discard_sql_snapshot_candidates(
        &mut self,
    ) -> Result<(), RuntimeError> {
        if let Some(pending) = self.pending_sql_snapshot_restore.take() {
            let provider = pending.candidate_sql.provider();
            let handles = pending.candidate_sql.cleanup_handles();
            for handle in handles {
                self.retain_sql_cleanup(provider, handle);
            }
        }
        if let Some(ready) = self.ready_sql_snapshot_restore.take() {
            let provider = ready.candidate_sql.provider();
            let handles = ready.candidate_sql.cleanup_handles();
            for handle in handles {
                self.retain_sql_cleanup(provider, handle);
            }
        }
        self.flush_sql_cleanup_queue().map(|_| ())
    }

    pub(in crate::session) fn begin_sql_snapshot_restore(
        &mut self,
        message_id: u64,
        bytes: Vec<u8>,
        connections: Vec<crate::runtime_snapshot::SqlConnectionSnapshot>,
    ) -> Result<(), RuntimeError> {
        if self.pending_sql_snapshot_restore.is_some() {
            return self.reject(
                message_id,
                CommandErrorCode::InvalidState,
                "another SQL snapshot restore is already active",
            );
        }
        if connections.len() > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_connections as usize
        {
            return self.reject(
                message_id,
                CommandErrorCode::ResourceLimit,
                "SQL snapshot exceeds the fixed connection limit",
            );
        }
        if let Err(message) = self.validate_exact_sql_restore(&connections) {
            return self.reject(message_id, CommandErrorCode::InvalidValue, message);
        }
        self.install_exact_sql_restore(message_id, bytes, connections)
    }

    pub(in crate::session) fn validate_exact_sql_restore(
        &self,
        connections: &[crate::runtime_snapshot::SqlConnectionSnapshot],
    ) -> Result<(), &'static str> {
        let mut names = BTreeSet::new();
        for snapshot in connections {
            let valid_identity = snapshot.identity.sqlite_version
                == era_runtime_protocol::SQL_SQLITE_VERSION
                && snapshot.identity.format_version
                    == era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION
                && snapshot.durable_revision.sha256.as_slice().len() == 32
                && match &snapshot.identity.source {
                    era_runtime_protocol::SqlDatabaseSourceV1::Memory => true,
                    era_runtime_protocol::SqlDatabaseSourceV1::ResourceSeed(seed) => {
                        seed.sha256.as_slice().len() == 32
                            && self.resource_seed_identity_available(seed)
                    }
                };
            let Some(key) = crate::sql::normalize_sql_name(&snapshot.logical_name) else {
                return Err("SQL state contains an invalid connection name");
            };
            if !valid_identity || !names.insert(key) {
                return Err("SQL state contains an invalid or duplicate database identity");
            }
        }
        Ok(())
    }

    fn resource_seed_identity_available(
        &self,
        seed: &era_runtime_protocol::SqlResourceSeedV1,
    ) -> bool {
        let Some(project) = self.project_snapshot.as_ref() else {
            return false;
        };
        let file = project.manifest.files.iter().find(|file| {
            file.category == FileCategory::Resource
                && file.relative_path.eq_ignore_ascii_case(&seed.resource_id)
        });
        let Some(file) = file else {
            return false;
        };
        let host_owned_payload_is_available = || {
            file.content_hash
                .as_ref()
                .is_some_and(|digest| digest.as_slice().len() == 32)
        };
        match &file.payload {
            FilePayload::Utf8(value) => {
                if value.is_empty() && host_owned_payload_is_available() {
                    true
                } else {
                    Sha256::digest(value.as_bytes()).as_slice() == seed.sha256.as_slice()
                }
            }
            FilePayload::Bytes(value) => {
                if value.as_slice().is_empty() && host_owned_payload_is_available() {
                    true
                } else {
                    Sha256::digest(value.as_slice()).as_slice() == seed.sha256.as_slice()
                }
            }
            // External bytes and released cache payloads stay host-owned. Their BLAKE3 content
            // hash binds the project/cache identity; the SQL provider verifies the saved SHA-256
            // against the actual seed before publishing an exact restore candidate.
            FilePayload::ExternalResource(_) => host_owned_payload_is_available(),
            FilePayload::IoError(_) => false,
        }
    }

    fn install_exact_sql_restore(
        &mut self,
        message_id: u64,
        bytes: Vec<u8>,
        connections: Vec<crate::runtime_snapshot::SqlConnectionSnapshot>,
    ) -> Result<(), RuntimeError> {
        let mut candidate_sql = self.sql.clone();
        candidate_sql.reset_for_project_boundary();
        self.pending_sql_snapshot_restore = Some(PendingSqlSnapshotRestore {
            message_id,
            bytes,
            candidate_sql,
            remaining: connections.into(),
        });
        if self.issue_next_sql_snapshot_open().is_err() {
            let message_id = self.discard_pending_sql_snapshot_restore()?;
            let rejected = self.reject(
                message_id,
                CommandErrorCode::ResourceLimit,
                "SQL exact restore request could not be emitted",
            );
            if rejected.is_ok() {
                let _ = self.flush_sql_cleanup_queue();
            }
            return rejected;
        }
        Ok(())
    }

    fn issue_next_sql_snapshot_open(&mut self) -> Result<(), RuntimeError> {
        let snapshot = self
            .pending_sql_snapshot_restore
            .as_mut()
            .and_then(|pending| pending.remaining.pop_front())
            .ok_or_else(|| RuntimeError::Internal("SQL snapshot restore queue is empty".into()))?;
        let key = crate::sql::normalize_sql_name(&snapshot.logical_name)
            .expect("snapshot names were prevalidated");
        let source = crate::sql::opening_source(&snapshot.identity);
        let pending = self
            .pending_sql_snapshot_restore
            .as_mut()
            .expect("restore candidate exists");
        let connection = pending
            .candidate_sql
            .reserve_open(key.clone(), source)
            .map_err(|_| {
                RuntimeError::Internal("prevalidated SQL restore reservation failed".into())
            })?;
        let provider = pending.candidate_sql.provider();
        let result = self.issue_sql_service_for(
            provider,
            SqlServiceContinuation::RestoreOpen {
                epoch: provider.service_epoch,
                connection,
                snapshot: snapshot.clone(),
            },
            era_runtime_protocol::SqlOperationV1::Open {
                connection,
                logical_name: snapshot.logical_name,
                identity: snapshot.identity,
                revision: era_runtime_protocol::SqlOpenRevisionV1::Exact(snapshot.durable_revision),
                limits: era_runtime_protocol::SqlLimitsV1::FIXED,
            },
        );
        if result.is_err()
            && let Some(pending) = self.pending_sql_snapshot_restore.as_mut()
        {
            pending.candidate_sql.release_open(&key, connection);
        }
        result
    }

    fn discard_pending_sql_snapshot_restore(&mut self) -> Result<u64, RuntimeError> {
        let pending = self.pending_sql_snapshot_restore.take().ok_or_else(|| {
            RuntimeError::Internal("SQL snapshot restore discard has no candidate".into())
        })?;
        let provider = pending.candidate_sql.provider();
        let handles = pending.candidate_sql.cleanup_handles();
        for handle in handles {
            self.retain_sql_cleanup(provider, handle);
        }
        Ok(pending.message_id)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn complete_sql_snapshot_open(
        &mut self,
        message_id: u64,
        service_request_id: u64,
        epoch: u64,
        connection: era_runtime_protocol::SqlConnectionHandleV1,
        snapshot: crate::runtime_snapshot::SqlConnectionSnapshot,
        result: ServiceResult,
    ) -> Result<(), RuntimeError> {
        let Some(pending) = self.pending_sql_snapshot_restore.as_ref() else {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "SQL snapshot open has no pending restore",
            );
        };
        let provider = pending.candidate_sql.provider();
        let key = crate::sql::normalize_sql_name(&snapshot.logical_name)
            .expect("snapshot name was prevalidated");
        if epoch != provider.service_epoch
            || !pending.candidate_sql.opening_matches(
                &key,
                &crate::sql::opening_source(&snapshot.identity),
                connection,
            )
        {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "SQL snapshot open belongs to a stale candidate",
            );
        }
        let reject_restore = |session: &mut Self, code, message| {
            let message_id = session.discard_pending_sql_snapshot_restore()?;
            let rejected = session.reject(message_id, code, message);
            if rejected.is_ok() {
                let _ = session.flush_sql_cleanup_queue();
            }
            rejected
        };
        let response: era_runtime_protocol::SqlResponseV1 = match result {
            ServiceResult::Ready { payload } => match decode_canonical(payload.as_slice()) {
                Ok(response) => response,
                Err(_) => {
                    return reject_restore(
                        self,
                        CommandErrorCode::InvalidValue,
                        "SQL exact restore response payload is malformed",
                    );
                }
            },
            ServiceResult::Error { .. } => {
                return reject_restore(
                    self,
                    CommandErrorCode::InvalidState,
                    "SQL provider transport failed during exact snapshot restore",
                );
            }
        };
        if response.provider != provider {
            self.operations.insert_service(
                service_request_id,
                PendingService::Sql(SqlServiceContinuation::RestoreOpen {
                    epoch,
                    connection,
                    snapshot,
                }),
            );
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "SQL snapshot response provider differs from its candidate",
            );
        }
        if let era_runtime_protocol::SqlResultV1::Error { error } = &response.result {
            if error.operation != era_runtime_protocol::SqlOperationKindV1::Open {
                return reject_restore(
                    self,
                    CommandErrorCode::InvalidValue,
                    "SQL snapshot provider returned an error for the wrong operation",
                );
            }
            return reject_restore(
                self,
                CommandErrorCode::InvalidValue,
                "an exact SQL database revision is unavailable",
            );
        }
        let (
            era_runtime_protocol::SqlResultV1::Opened {
                sqlite_version,
                limits,
            },
            Some(database),
            None,
        ) = (&response.result, &response.database, &response.reader)
        else {
            return reject_restore(
                self,
                CommandErrorCode::InvalidValue,
                "SQL exact restore response shape is invalid",
            );
        };
        if sqlite_version != era_runtime_protocol::SQL_SQLITE_VERSION
            || *limits != era_runtime_protocol::SqlLimitsV1::FIXED
            || database.connection != connection
            || !database.connected
            || database.transaction_active
            || database.durable_revision.as_ref() != Some(&snapshot.durable_revision)
        {
            return reject_restore(
                self,
                CommandErrorCode::VersionMismatch,
                "SQL provider did not reopen the exact snapshot revision",
            );
        }
        let resource_digest =
            crate::sql::database_source_resource(&snapshot.identity).and_then(|seed| {
                self.project_snapshot
                    .as_ref()?
                    .resources
                    .iter()
                    .find(|resource| {
                        resource.category == FileCategory::Resource
                            && resource
                                .relative_path
                                .eq_ignore_ascii_case(&seed.resource_id)
                    })
                    .map(|resource| resource.payload_digest)
            });
        let pending = self
            .pending_sql_snapshot_restore
            .as_mut()
            .expect("restore candidate was checked");
        if !pending.candidate_sql.release_open(&key, connection)
            || !pending
                .candidate_sql
                .insert_connection(crate::sql::SqlConnection {
                    logical_name: snapshot.logical_name,
                    identity: snapshot.identity,
                    handle: connection,
                    resource_digest,
                    transaction_active: false,
                    durable_revision: Some(snapshot.durable_revision),
                })
        {
            return reject_restore(
                self,
                CommandErrorCode::InvalidValue,
                "SQL snapshot connection reservation became inconsistent",
            );
        }
        if !pending.remaining.is_empty() {
            if self.issue_next_sql_snapshot_open().is_err() {
                return reject_restore(
                    self,
                    CommandErrorCode::ResourceLimit,
                    "the next SQL exact restore request could not be emitted",
                );
            }
            return Ok(());
        }
        let pending = self
            .pending_sql_snapshot_restore
            .take()
            .expect("restore candidate exists");
        let message_id = pending.message_id;
        let bytes = pending.bytes;
        let digest = *blake3::hash(&bytes).as_bytes();
        self.ready_sql_snapshot_restore = Some(ReadySqlSnapshotRestore {
            digest,
            candidate_sql: pending.candidate_sql,
        });
        self.start_vm_snapshot(message_id, &bytes)
    }
}
