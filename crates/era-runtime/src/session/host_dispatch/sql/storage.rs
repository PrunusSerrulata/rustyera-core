//! Resource seed and MAP XML storage completions for SQL host requests.

use sha2::{Digest as _, Sha256};

#[allow(clippy::wildcard_imports)]
use super::super::super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines)]
    pub(in crate::session) fn complete_sql_storage(
        &mut self,
        message_id: u64,
        pending: PendingStorage,
        result: StorageResult,
    ) -> Result<(), RuntimeError> {
        match pending {
            PendingStorage::SqlSeedRead {
                request,
                epoch,
                connection_key,
                logical_name,
                connection,
                path,
                expected_digest,
            } => {
                if epoch != self.sql.service_epoch() {
                    return self.reject(
                        message_id,
                        CommandErrorCode::StaleRequest,
                        "SQL seed response belongs to an old service epoch",
                    );
                }
                let data = match result {
                    StorageResult::Read { data, .. } => data,
                    StorageResult::Error { error } => {
                        self.sql.release_open(&connection_key, connection);
                        return self.finish_sql_script_fault(
                            request,
                            erabasic_vm::ScriptFaultKind::Operation,
                            format!("SQL seed Resource read failed: {:?}", error.kind),
                        );
                    }
                    _ => {
                        self.sql.release_open(&connection_key, connection);
                        return self.fault(
                            FaultCode::ServiceFailure,
                            "storage response kind differs from SQL seed read",
                            None,
                        );
                    }
                };
                if u64::try_from(data.as_slice().len()).expect("Resource byte length fits u64")
                    > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_database_bytes
                    || blake3::hash(data.as_slice()).as_bytes() != &expected_digest
                {
                    self.sql.release_open(&connection_key, connection);
                    return self.finish_sql_script_fault(
                        request,
                        erabasic_vm::ScriptFaultKind::Operation,
                        "SQL seed Resource size or project identity differs",
                    );
                }
                let identity = Self::sql_seed_identity(path, data.as_slice());
                let result = self.issue_sql_service(
                    SqlServiceContinuation::Open {
                        request,
                        epoch,
                        logical_name: logical_name.clone(),
                        connection,
                        identity: identity.clone(),
                        resource_digest: Some(expected_digest),
                    },
                    era_runtime_protocol::SqlOperationV1::Open {
                        connection,
                        logical_name,
                        identity,
                        revision: era_runtime_protocol::SqlOpenRevisionV1::Current,
                        limits: era_runtime_protocol::SqlLimitsV1::FIXED,
                    },
                );
                if result.is_err() {
                    self.sql.release_open(&connection_key, connection);
                }
                result
            }
            PendingStorage::SqlMapXmlRead {
                request,
                epoch,
                connection_key,
                connection,
                table,
                path: _,
                expected_digest,
            } => {
                if epoch != self.sql.service_epoch()
                    || self
                        .sql
                        .connection_by_key(&connection_key)
                        .is_none_or(|value| value.handle != connection)
                {
                    return self.reject(
                        message_id,
                        CommandErrorCode::StaleRequest,
                        "SQL MAP response belongs to a stale connection",
                    );
                }
                let data = match result {
                    StorageResult::Read { data, .. } => data,
                    StorageResult::Error { error } => {
                        self.sql.release_connection(&connection_key);
                        return self.finish_sql_script_fault(
                            request,
                            erabasic_vm::ScriptFaultKind::Operation,
                            format!("SQL MAP Resource read failed: {:?}", error.kind),
                        );
                    }
                    _ => {
                        self.sql.release_connection(&connection_key);
                        return self.fault(
                            FaultCode::ServiceFailure,
                            "storage response kind differs from SQL MAP XML read",
                            None,
                        );
                    }
                };
                let limits = era_runtime_protocol::SqlLimitsV1::FIXED;
                if u64::try_from(data.as_slice().len()).expect("Resource byte length fits u64")
                    > limits.maximum_map_bytes
                    || blake3::hash(data.as_slice()).as_bytes() != &expected_digest
                {
                    self.sql.release_connection(&connection_key);
                    return self.finish_sql_script_fault(
                        request,
                        erabasic_vm::ScriptFaultKind::Bounds,
                        "SQL MAP XML size or project identity differs",
                    );
                }
                let Ok(text) = std::str::from_utf8(data.as_slice()) else {
                    self.sql.release_connection(&connection_key);
                    return self.finish_sql_script_fault(
                        request,
                        erabasic_vm::ScriptFaultKind::Parse,
                        "SQL MAP XML is not UTF-8",
                    );
                };
                let rows = match erabasic_vm::parse_map_xml_rows(text) {
                    Ok(rows) => rows,
                    Err(error) => {
                        self.sql.release_connection(&connection_key);
                        return self.finish_sql_script_fault(
                            request,
                            erabasic_vm::ScriptFaultKind::Parse,
                            format!("SQL MAP XML is invalid: {error}"),
                        );
                    }
                };
                let retained = rows.iter().try_fold(0_u64, |total, (key, value)| {
                    total
                        .checked_add(key.len() as u64)?
                        .checked_add(value.len() as u64)
                });
                if rows.len() > limits.maximum_map_rows as usize
                    || retained.is_none_or(|bytes| bytes > limits.maximum_map_bytes)
                {
                    self.sql.release_connection(&connection_key);
                    return self.finish_sql_script_fault(
                        request,
                        erabasic_vm::ScriptFaultKind::Bounds,
                        "SQL MAP rows exceed the fixed row or byte limit",
                    );
                }
                let expected_rows =
                    u32::try_from(rows.len()).expect("validated SQL MAP row count fits u32");
                let result = self.issue_sql_service(
                    SqlServiceContinuation::ImportMap {
                        request,
                        epoch,
                        connection_key: connection_key.clone(),
                        connection,
                        expected_rows,
                    },
                    era_runtime_protocol::SqlOperationV1::ImportMapRows {
                        connection,
                        table,
                        rows: rows
                            .into_iter()
                            .map(|(key, value)| era_runtime_protocol::SqlMapRowV1 { key, value })
                            .collect(),
                    },
                );
                if result.is_err() {
                    self.sql.release_connection(&connection_key);
                }
                result
            }
            _ => Err(RuntimeError::Internal(
                "not a SQL storage continuation".into(),
            )),
        }
    }

    pub(in crate::session) fn sql_seed_identity(
        path: String,
        bytes: &[u8],
    ) -> era_runtime_protocol::SqlDatabaseIdentityV1 {
        era_runtime_protocol::SqlDatabaseIdentityV1 {
            source: era_runtime_protocol::SqlDatabaseSourceV1::ResourceSeed(
                era_runtime_protocol::SqlResourceSeedV1 {
                    resource_id: path,
                    sha256: ProtocolBytes::new(Sha256::digest(bytes).to_vec()),
                },
            ),
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            format_version: era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION,
        }
    }
}
