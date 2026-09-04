//! Validation and application of untrusted safe-SQL provider completions.

#[allow(clippy::wildcard_imports)]
use super::super::super::*;

impl RuntimeSession {
    #[allow(clippy::too_many_lines, clippy::single_match_else)]
    pub(in crate::session) fn complete_sql_service(
        &mut self,
        message_id: u64,
        service_request_id: u64,
        continuation: SqlServiceContinuation,
        result: ServiceResult,
    ) -> Result<(), RuntimeError> {
        if let SqlServiceContinuation::CleanupDisconnect {
            provider,
            connection,
            ..
        } = &continuation
        {
            return match result {
                ServiceResult::Ready { payload } => {
                    let response: era_runtime_protocol::SqlResponseV1 =
                        match decode_canonical(payload.as_slice()) {
                            Ok(response) => response,
                            Err(_) => {
                                self.retain_sql_cleanup(*provider, *connection);
                                self.emit_log(
                                    RuntimeLogLevel::Warning,
                                    "SQL cleanup response payload was malformed",
                                )?;
                                return Ok(());
                            }
                        };
                    let valid_result = matches!(
                        response.result,
                        era_runtime_protocol::SqlResultV1::Disconnected
                    ) || matches!(
                        response.result,
                        era_runtime_protocol::SqlResultV1::Error { ref error }
                            if error.operation == era_runtime_protocol::SqlOperationKindV1::Disconnect
                    );
                    let valid_database = response.database.as_ref().is_none_or(|database| {
                        database.connection == *connection && !database.connected
                    });
                    if response.provider != *provider || !valid_result || !valid_database {
                        self.retain_sql_cleanup(*provider, *connection);
                        self.emit_log(
                            RuntimeLogLevel::Warning,
                            "SQL cleanup response did not confirm the expected disconnect",
                        )?;
                    }
                    Ok(())
                }
                ServiceResult::Error { error } => {
                    self.retain_sql_cleanup(*provider, *connection);
                    self.emit_log(
                        RuntimeLogLevel::Warning,
                        format!(
                            "SQL cleanup transport failure: {}: {}",
                            error.code, error.message
                        ),
                    )?;
                    Ok(())
                }
            };
        }
        let continuation = match continuation {
            SqlServiceContinuation::RestoreOpen {
                epoch,
                connection,
                snapshot,
            } => {
                return self.complete_sql_snapshot_open(
                    message_id,
                    service_request_id,
                    epoch,
                    connection,
                    snapshot,
                    result,
                );
            }
            continuation => continuation,
        };
        if continuation_epoch(&continuation) != self.sql.service_epoch() {
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "SQL response belongs to an old service epoch",
            );
        }
        let response: era_runtime_protocol::SqlResponseV1 = match result {
            ServiceResult::Ready { payload } => match decode_canonical(payload.as_slice()) {
                Ok(response) => response,
                Err(_) => {
                    self.release_sql_continuation(&continuation);
                    self.cleanup_uncertain_sql_open(&continuation);
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "SQL service response payload is malformed",
                        None,
                    );
                }
            },
            ServiceResult::Error { error } => {
                self.release_sql_continuation(&continuation);
                self.cleanup_uncertain_sql_open(&continuation);
                return self.fault(
                    FaultCode::ServiceFailure,
                    &format!(
                        "SQL service transport failure: {}: {}",
                        error.code, error.message
                    ),
                    None,
                );
            }
        };
        if response.provider != self.sql.provider() {
            self.operations
                .insert_service(service_request_id, PendingService::Sql(continuation));
            return self.reject(
                message_id,
                CommandErrorCode::StaleRequest,
                "SQL response provider differs from its pending request",
            );
        }

        let reader_connection_key =
            continuation_reader(&continuation).and_then(|(reader_id, _)| {
                self.sql
                    .reader(reader_id)
                    .map(|reader| reader.connection.clone())
            });
        let expected_connection = continuation_connection(&continuation).or_else(|| {
            reader_connection_key.as_deref().and_then(|key| {
                self.sql
                    .connection_by_key(key)
                    .map(|connection| connection.handle)
            })
        });
        if let Err(message) = validate_sql_response(
            &continuation,
            &response,
            expected_connection,
            continuation_reader(&continuation).and_then(|(id, _)| self.sql.reader(id)),
        ) {
            self.release_sql_continuation(&continuation);
            self.cleanup_uncertain_sql_open(&continuation);
            return self.fault(FaultCode::ServiceFailure, message, None);
        }
        if let Some(database) = &response.database
            && let Some(key) =
                continuation_connection_key(&continuation).or(reader_connection_key.as_deref())
            && let Some(connection) = self.sql.connection_mut(key)
        {
            connection.transaction_active = database.transaction_active;
            connection
                .durable_revision
                .clone_from(&database.durable_revision);
            if !database.connected {
                self.sql.remove_connection(key);
            }
        }
        if let Some(reader) = &response.reader {
            if matches!(
                &continuation,
                SqlServiceContinuation::Execute {
                    mode: era_runtime_protocol::SqlExecuteModeV1::Reader,
                    ..
                }
            ) {
                // ReaderOpened introduces its provider handle below.
            } else if let Some((reader_id, _)) = continuation_reader(&continuation)
                && let Some(state) = self.sql.reader_mut(reader_id)
            {
                state.status = reader.status;
                state.rows_read = reader.rows_read;
            }
        }

        if let era_runtime_protocol::SqlResultV1::Error { error } = &response.result {
            self.release_sql_continuation(&continuation);
            if response
                .database
                .as_ref()
                .is_some_and(|database| database.connected)
            {
                self.cleanup_uncertain_sql_open(&continuation);
            }
            let mut context = error.context.clone();
            context
                .sort_by(|left, right| left.key.cmp(&right.key).then(left.value.cmp(&right.value)));
            let context = context
                .into_iter()
                .map(|entry| {
                    format!(
                        "{}:{}={}:{}",
                        entry.key.len(),
                        entry.key,
                        entry.value.len(),
                        entry.value
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            return self.complete_sql_continuation_fault(
                &continuation,
                sql_fault_kind(error.code),
                format!(
                    "rustyera.sql/{}/{}:{context}",
                    sql_operation_name(error.operation),
                    sql_error_code_name(error.code)
                ),
            );
        }

        match (continuation, response.result) {
            (
                SqlServiceContinuation::Open {
                    request,
                    logical_name,
                    connection,
                    identity,
                    resource_digest,
                    ..
                },
                era_runtime_protocol::SqlResultV1::Opened {
                    sqlite_version,
                    limits,
                },
            ) => {
                let key = crate::sql::normalize_sql_name(&logical_name)
                    .expect("open continuation name was validated");
                if !self.sql.release_open(&key, connection) {
                    self.cleanup_uncertain_sql_open(&SqlServiceContinuation::Open {
                        request,
                        epoch: self.sql.service_epoch(),
                        logical_name: logical_name.clone(),
                        connection,
                        identity: identity.clone(),
                        resource_digest,
                    });
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "SQL open reservation is missing",
                        None,
                    );
                }
                let database = response.database.ok_or_else(|| {
                    RuntimeError::Internal("SQL Opened response omitted database state".into())
                })?;
                if sqlite_version != era_runtime_protocol::SQL_SQLITE_VERSION
                    || limits != era_runtime_protocol::SqlLimitsV1::FIXED
                    || database.connection != connection
                    || !database.connected
                {
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "SQL provider identity or fixed limits differ",
                        None,
                    );
                }
                if !self.sql.insert_connection(crate::sql::SqlConnection {
                    logical_name: logical_name.clone(),
                    identity: identity.clone(),
                    handle: connection,
                    resource_digest,
                    transaction_active: database.transaction_active,
                    durable_revision: database.durable_revision,
                }) {
                    self.cleanup_uncertain_sql_open(&SqlServiceContinuation::Open {
                        request,
                        epoch: self.sql.service_epoch(),
                        logical_name,
                        connection,
                        identity,
                        resource_digest,
                    });
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "SQL provider opened a duplicate connection",
                        None,
                    );
                }
                self.finish_sql_value(request, VmValue::Integer(0))
            }
            (
                SqlServiceContinuation::Execute {
                    request,
                    connection_key,
                    mode: era_runtime_protocol::SqlExecuteModeV1::NonQuery,
                    ..
                },
                era_runtime_protocol::SqlResultV1::NonQuery { affected_rows },
            ) => {
                self.sql.release_connection(&connection_key);
                self.finish_sql_value(request, VmValue::Integer(affected_rows))
            }
            (
                SqlServiceContinuation::Execute {
                    request,
                    connection_key,
                    mode: era_runtime_protocol::SqlExecuteModeV1::ScalarInteger,
                    ..
                },
                era_runtime_protocol::SqlResultV1::Scalar { value },
            ) => {
                self.sql.release_connection(&connection_key);
                let value = match value {
                    era_runtime_protocol::SqlValueV1::Null => 0,
                    era_runtime_protocol::SqlValueV1::Integer(value) => value,
                    era_runtime_protocol::SqlValueV1::String(_) => {
                        return self.finish_sql_script_fault(
                            request,
                            erabasic_vm::ScriptFaultKind::Operation,
                            "SQL scalar value is not an integer",
                        );
                    }
                };
                self.finish_sql_value(request, VmValue::Integer(value))
            }
            (
                SqlServiceContinuation::Execute {
                    request,
                    connection_key,
                    mode: era_runtime_protocol::SqlExecuteModeV1::ScalarString,
                    ..
                },
                era_runtime_protocol::SqlResultV1::Scalar { value },
            ) => {
                self.sql.release_connection(&connection_key);
                let value = match value {
                    era_runtime_protocol::SqlValueV1::Null => String::new(),
                    era_runtime_protocol::SqlValueV1::Integer(value) => value.to_string(),
                    era_runtime_protocol::SqlValueV1::String(value) => value,
                };
                self.finish_sql_value(request, VmValue::String(value))
            }
            (
                SqlServiceContinuation::Execute {
                    request,
                    connection_key,
                    connection: _,
                    mode: era_runtime_protocol::SqlExecuteModeV1::Reader,
                    reader_id: Some(reader_id),
                    ..
                },
                era_runtime_protocol::SqlResultV1::ReaderOpened { reader },
            ) => {
                self.sql.release_connection(&connection_key);
                let reader_state = response.reader.ok_or_else(|| {
                    RuntimeError::Internal("SQL ReaderOpened response omitted reader state".into())
                })?;
                if reader.service_epoch != self.sql.service_epoch()
                    || reader_state.reader != reader
                    || !self.sql.insert_reader(
                        reader_id,
                        crate::sql::SqlReader {
                            connection: connection_key,
                            handle: reader,
                            status: reader_state.status,
                            rows_read: reader_state.rows_read,
                        },
                    )
                {
                    return self.fault(
                        FaultCode::ServiceFailure,
                        "SQL provider returned an invalid reader handle",
                        None,
                    );
                }
                self.finish_sql_value(request, VmValue::Integer(reader_id))
            }
            (
                SqlServiceContinuation::ReaderRead {
                    request, reader_id, ..
                },
                era_runtime_protocol::SqlResultV1::ReaderAdvanced { has_row },
            ) => {
                let key = self
                    .sql
                    .reader(reader_id)
                    .map(|reader| reader.connection.clone());
                if let Some(key) = key {
                    self.sql.release_connection(&key);
                }
                self.finish_sql_value(request, VmValue::Integer(i64::from(has_row)))
            }
            (
                SqlServiceContinuation::ReaderGet {
                    request,
                    reader_id,
                    string,
                    ..
                },
                era_runtime_protocol::SqlResultV1::ReaderValue { value },
            ) => {
                let key = self
                    .sql
                    .reader(reader_id)
                    .map(|reader| reader.connection.clone());
                if let Some(key) = key {
                    self.sql.release_connection(&key);
                }
                let value = match (string, value) {
                    (false, era_runtime_protocol::SqlValueV1::Null) => VmValue::Integer(0),
                    (false, era_runtime_protocol::SqlValueV1::Integer(value)) => {
                        VmValue::Integer(value)
                    }
                    (true, era_runtime_protocol::SqlValueV1::Null) => {
                        VmValue::String(String::new())
                    }
                    (true, era_runtime_protocol::SqlValueV1::String(value)) => {
                        VmValue::String(value)
                    }
                    (true, era_runtime_protocol::SqlValueV1::Integer(value)) => {
                        VmValue::String(value.to_string())
                    }
                    (false, era_runtime_protocol::SqlValueV1::String(_)) => {
                        return self.finish_sql_script_fault(
                            request,
                            erabasic_vm::ScriptFaultKind::Operation,
                            "SQL reader value is not an integer",
                        );
                    }
                };
                self.finish_sql_value(request, value)
            }
            (
                SqlServiceContinuation::ReaderIsNull {
                    request, reader_id, ..
                },
                era_runtime_protocol::SqlResultV1::ReaderNull { is_null },
            ) => {
                let key = self
                    .sql
                    .reader(reader_id)
                    .map(|reader| reader.connection.clone());
                if let Some(key) = key {
                    self.sql.release_connection(&key);
                }
                self.finish_sql_value(request, VmValue::Integer(i64::from(is_null)))
            }
            (
                SqlServiceContinuation::ReaderClose {
                    request, reader_id, ..
                },
                era_runtime_protocol::SqlResultV1::ReaderClosed,
            ) => {
                let key = self
                    .sql
                    .remove_reader(reader_id)
                    .map(|reader| reader.connection);
                if let Some(key) = key {
                    self.sql.release_connection(&key);
                }
                self.finish_sql_value(request, VmValue::Integer(1))
            }
            (
                SqlServiceContinuation::ImportMap {
                    request,
                    connection_key,
                    ..
                },
                era_runtime_protocol::SqlResultV1::MapImported { .. },
            ) => {
                self.sql.release_connection(&connection_key);
                self.finish_sql_value(request, VmValue::Integer(1))
            }
            (
                SqlServiceContinuation::Disconnect {
                    request,
                    connection_key,
                    ..
                },
                era_runtime_protocol::SqlResultV1::Disconnected,
            ) => {
                self.sql.remove_connection(&connection_key);
                self.finish_sql_value(request, VmValue::Integer(1))
            }
            (continuation, _) => {
                self.release_sql_continuation(&continuation);
                self.fault(
                    FaultCode::ServiceFailure,
                    "SQL response result differs from its continuation",
                    None,
                )
            }
        }
    }

    fn finish_sql_value(
        &mut self,
        request: erabasic_vm::HostRequestId,
        value: VmValue,
    ) -> Result<(), RuntimeError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| RuntimeError::Internal("pending SQL continuation has no VM".into()))?;
        commit_completion(
            vm,
            request,
            VmHostCompletion::Ready(HostReady {
                value: Some(value),
                writes: Vec::new(),
            }),
        )?;
        self.set_phase(RuntimePhase::Running)
    }

    fn complete_sql_continuation_fault(
        &mut self,
        continuation: &SqlServiceContinuation,
        kind: erabasic_vm::ScriptFaultKind,
        message: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.finish_sql_script_fault(continuation_request(continuation), kind, message)
    }

    fn release_sql_continuation(&mut self, continuation: &SqlServiceContinuation) {
        match continuation {
            SqlServiceContinuation::Open {
                logical_name,
                connection,
                ..
            } => {
                if let Some(key) = crate::sql::normalize_sql_name(logical_name) {
                    self.sql.release_open(&key, *connection);
                }
            }
            SqlServiceContinuation::RestoreOpen { .. }
            | SqlServiceContinuation::CleanupDisconnect { .. } => {}
            SqlServiceContinuation::Execute { connection_key, .. }
            | SqlServiceContinuation::ImportMap { connection_key, .. }
            | SqlServiceContinuation::Disconnect { connection_key, .. } => {
                self.sql.release_connection(connection_key);
            }
            SqlServiceContinuation::ReaderRead { reader_id, .. }
            | SqlServiceContinuation::ReaderGet { reader_id, .. }
            | SqlServiceContinuation::ReaderIsNull { reader_id, .. }
            | SqlServiceContinuation::ReaderClose { reader_id, .. } => {
                let key = self
                    .sql
                    .reader(*reader_id)
                    .map(|reader| reader.connection.clone());
                if let Some(key) = key {
                    self.sql.release_connection(&key);
                }
            }
        }
    }

    fn cleanup_uncertain_sql_open(&mut self, continuation: &SqlServiceContinuation) {
        let SqlServiceContinuation::Open { connection, .. } = continuation else {
            return;
        };
        let _ = self.emit_sql_cleanup_for(self.sql.provider(), std::slice::from_ref(connection));
    }
}

fn continuation_request(value: &SqlServiceContinuation) -> erabasic_vm::HostRequestId {
    match value {
        SqlServiceContinuation::Open { request, .. }
        | SqlServiceContinuation::Execute { request, .. }
        | SqlServiceContinuation::ReaderRead { request, .. }
        | SqlServiceContinuation::ReaderGet { request, .. }
        | SqlServiceContinuation::ReaderIsNull { request, .. }
        | SqlServiceContinuation::ReaderClose { request, .. }
        | SqlServiceContinuation::ImportMap { request, .. }
        | SqlServiceContinuation::Disconnect { request, .. } => *request,
        SqlServiceContinuation::RestoreOpen { .. } => {
            unreachable!("restore continuations reject through the state-import request")
        }
        SqlServiceContinuation::CleanupDisconnect { .. } => {
            unreachable!("cleanup continuations do not resume a VM request")
        }
    }
}

fn continuation_epoch(value: &SqlServiceContinuation) -> u64 {
    match value {
        SqlServiceContinuation::Open { epoch, .. }
        | SqlServiceContinuation::Execute { epoch, .. }
        | SqlServiceContinuation::ReaderRead { epoch, .. }
        | SqlServiceContinuation::ReaderGet { epoch, .. }
        | SqlServiceContinuation::ReaderIsNull { epoch, .. }
        | SqlServiceContinuation::ReaderClose { epoch, .. }
        | SqlServiceContinuation::ImportMap { epoch, .. }
        | SqlServiceContinuation::Disconnect { epoch, .. }
        | SqlServiceContinuation::RestoreOpen { epoch, .. }
        | SqlServiceContinuation::CleanupDisconnect { epoch, .. } => *epoch,
    }
}

fn continuation_connection(
    value: &SqlServiceContinuation,
) -> Option<era_runtime_protocol::SqlConnectionHandleV1> {
    match value {
        SqlServiceContinuation::Open { connection, .. }
        | SqlServiceContinuation::Execute { connection, .. }
        | SqlServiceContinuation::ImportMap { connection, .. }
        | SqlServiceContinuation::Disconnect { connection, .. }
        | SqlServiceContinuation::RestoreOpen { connection, .. }
        | SqlServiceContinuation::CleanupDisconnect { connection, .. } => Some(*connection),
        SqlServiceContinuation::ReaderRead { .. }
        | SqlServiceContinuation::ReaderGet { .. }
        | SqlServiceContinuation::ReaderIsNull { .. }
        | SqlServiceContinuation::ReaderClose { .. } => None,
    }
}

fn continuation_connection_key(value: &SqlServiceContinuation) -> Option<&str> {
    match value {
        SqlServiceContinuation::Execute { connection_key, .. }
        | SqlServiceContinuation::ImportMap { connection_key, .. }
        | SqlServiceContinuation::Disconnect { connection_key, .. } => Some(connection_key),
        _ => None,
    }
}

fn continuation_reader(
    value: &SqlServiceContinuation,
) -> Option<(i64, era_runtime_protocol::SqlReaderHandleV1)> {
    match value {
        SqlServiceContinuation::ReaderRead {
            reader_id, reader, ..
        }
        | SqlServiceContinuation::ReaderGet {
            reader_id, reader, ..
        }
        | SqlServiceContinuation::ReaderIsNull {
            reader_id, reader, ..
        }
        | SqlServiceContinuation::ReaderClose {
            reader_id, reader, ..
        } => Some((*reader_id, *reader)),
        _ => None,
    }
}

fn sql_fault_kind(code: era_runtime_protocol::SqlErrorCodeV1) -> erabasic_vm::ScriptFaultKind {
    use era_runtime_protocol::SqlErrorCodeV1 as Code;
    match code {
        Code::InvalidName
        | Code::InvalidSource
        | Code::InvalidConnectionString
        | Code::InvalidTableName
        | Code::InvalidRequest => erabasic_vm::ScriptFaultKind::Argument,
        Code::ColumnOutOfRange
        | Code::ConnectionLimit
        | Code::ReaderLimit
        | Code::SqlTooLarge
        | Code::ParameterLimit
        | Code::ParameterBytesLimit
        | Code::CellTooLarge
        | Code::DatabaseTooLarge
        | Code::MapRowLimit
        | Code::MapBytesLimit
        | Code::ReaderRowLimit => erabasic_vm::ScriptFaultKind::Bounds,
        _ => erabasic_vm::ScriptFaultKind::Operation,
    }
}

fn sql_operation_name(value: era_runtime_protocol::SqlOperationKindV1) -> &'static str {
    use era_runtime_protocol::SqlOperationKindV1 as Kind;
    match value {
        Kind::Open => "open",
        Kind::Execute => "execute",
        Kind::ReaderRead => "reader_read",
        Kind::ReaderGet => "reader_get",
        Kind::ReaderIsNull => "reader_is_null",
        Kind::ReaderClose => "reader_close",
        Kind::ImportMapRows => "import_map_rows",
        Kind::Disconnect => "disconnect",
    }
}

fn sql_error_code_name(value: era_runtime_protocol::SqlErrorCodeV1) -> &'static str {
    use era_runtime_protocol::SqlErrorCodeV1 as Code;
    match value {
        Code::InvalidRequest => "invalid_request",
        Code::InvalidName => "invalid_name",
        Code::InvalidSource => "invalid_source",
        Code::InvalidConnectionString => "invalid_connection_string",
        Code::ConnectionLimit => "connection_limit",
        Code::ConnectionConflict => "connection_conflict",
        Code::ConnectionNotFound => "connection_not_found",
        Code::ReaderLimit => "reader_limit",
        Code::ReaderNotFound => "reader_not_found",
        Code::ColumnOutOfRange => "column_out_of_range",
        Code::TypeMismatch => "type_mismatch",
        Code::SqlTooLarge => "sql_too_large",
        Code::ParameterLimit => "parameter_limit",
        Code::ParameterBytesLimit => "parameter_bytes_limit",
        Code::CellTooLarge => "cell_too_large",
        Code::DatabaseTooLarge => "database_too_large",
        Code::MapRowLimit => "map_row_limit",
        Code::MapBytesLimit => "map_bytes_limit",
        Code::ReaderRowLimit => "reader_row_limit",
        Code::ExecutionTimeout => "execution_timeout",
        Code::TransactionActive => "transaction_active",
        Code::RevisionConflict => "revision_conflict",
        Code::RevisionMissing => "revision_missing",
        Code::StorageFailure => "storage_failure",
        Code::Sqlite => "sqlite",
        Code::Cancelled => "cancelled",
        Code::StaleEpoch => "stale_epoch",
        Code::InvalidTableName => "invalid_table_name",
        Code::InvalidState => "invalid_state",
        Code::Unsupported => "unsupported",
    }
}

fn continuation_operation(
    value: &SqlServiceContinuation,
) -> era_runtime_protocol::SqlOperationKindV1 {
    use era_runtime_protocol::SqlOperationKindV1 as Kind;
    match value {
        SqlServiceContinuation::Open { .. } | SqlServiceContinuation::RestoreOpen { .. } => {
            Kind::Open
        }
        SqlServiceContinuation::Execute { .. } => Kind::Execute,
        SqlServiceContinuation::ReaderRead { .. } => Kind::ReaderRead,
        SqlServiceContinuation::ReaderGet { .. } => Kind::ReaderGet,
        SqlServiceContinuation::ReaderIsNull { .. } => Kind::ReaderIsNull,
        SqlServiceContinuation::ReaderClose { .. } => Kind::ReaderClose,
        SqlServiceContinuation::ImportMap { .. } => Kind::ImportMapRows,
        SqlServiceContinuation::Disconnect { .. }
        | SqlServiceContinuation::CleanupDisconnect { .. } => Kind::Disconnect,
    }
}

#[allow(clippy::too_many_lines)]
fn validate_sql_response(
    continuation: &SqlServiceContinuation,
    response: &era_runtime_protocol::SqlResponseV1,
    expected_connection: Option<era_runtime_protocol::SqlConnectionHandleV1>,
    current_reader: Option<&crate::sql::SqlReader>,
) -> Result<(), &'static str> {
    use era_runtime_protocol::{SqlReaderStatusV1 as Status, SqlResultV1 as Result};

    let semantic_error = matches!(response.result, Result::Error { .. });
    if let Result::Error { error } = &response.result {
        if error.operation != continuation_operation(continuation) {
            return Err("SQL error operation differs from its continuation");
        }
        let limits = era_runtime_protocol::SqlLimitsV1::FIXED;
        let context_bytes = error.context.iter().try_fold(0_u64, |total, entry| {
            if entry.key.len() > limits.maximum_cell_bytes as usize
                || entry.value.len() > limits.maximum_cell_bytes as usize
            {
                return None;
            }
            total
                .checked_add(entry.key.len() as u64)?
                .checked_add(entry.value.len() as u64)
        });
        if error.context.len() > limits.maximum_parameters as usize
            || context_bytes.is_none_or(|bytes| bytes > limits.maximum_parameter_bytes)
            || error
                .sqlite_message
                .as_ref()
                .is_some_and(|message| message.len() > limits.maximum_cell_bytes as usize)
        {
            return Err("SQL error diagnostic context exceeds fixed limits");
        }
    }
    if !semantic_error && response.database.is_none() {
        return Err("SQL success response omitted authoritative database state");
    }
    if let Some(database) = &response.database {
        if expected_connection != Some(database.connection) {
            return Err("SQL response database handle differs from its continuation");
        }
        if database
            .durable_revision
            .as_ref()
            .is_some_and(|revision| revision.sha256.as_slice().len() != 32)
        {
            return Err("SQL response durable revision is not SHA-256");
        }
        if !semantic_error {
            let should_be_connected =
                !matches!(continuation, SqlServiceContinuation::Disconnect { .. });
            if database.connected != should_be_connected {
                return Err("SQL response connected state differs from its operation");
            }
            if matches!(continuation, SqlServiceContinuation::Open { .. })
                && database.transaction_active
            {
                return Err("SQL open response unexpectedly has an active transaction");
            }
        }
    }

    let expected_reader = continuation_reader(continuation).map(|(_, handle)| handle);
    if let Some(reader) = &response.reader {
        if let Some(expected) = expected_reader {
            if reader.reader != expected {
                return Err("SQL response reader handle differs from its continuation");
            }
        } else if !matches!(
            continuation,
            SqlServiceContinuation::Execute {
                mode: era_runtime_protocol::SqlExecuteModeV1::Reader,
                ..
            }
        ) {
            return Err("SQL response unexpectedly contains reader state");
        }
        if reader.rows_read > era_runtime_protocol::SqlLimitsV1::FIXED.maximum_reader_rows {
            return Err("SQL reader row limit exceeded");
        }
        if let Some(current) = current_reader
            && reader.rows_read < current.rows_read
        {
            return Err("SQL reader rows_read moved backwards");
        }
    }

    if semantic_error {
        return Ok(());
    }
    match (continuation, &response.result, &response.reader) {
        (
            SqlServiceContinuation::Open { .. },
            Result::Opened {
                sqlite_version,
                limits,
            },
            None,
        ) if sqlite_version == era_runtime_protocol::SQL_SQLITE_VERSION
            && *limits == era_runtime_protocol::SqlLimitsV1::FIXED =>
        {
            Ok(())
        }
        (
            SqlServiceContinuation::Execute {
                mode: era_runtime_protocol::SqlExecuteModeV1::NonQuery,
                ..
            },
            Result::NonQuery { affected_rows },
            None,
        ) if *affected_rows >= 0 => Ok(()),
        (
            SqlServiceContinuation::Execute {
                mode: era_runtime_protocol::SqlExecuteModeV1::ScalarInteger,
                ..
            },
            Result::Scalar { value },
            None,
        ) if sql_value_size_valid(value) => Ok(()),
        (
            SqlServiceContinuation::Execute {
                mode: era_runtime_protocol::SqlExecuteModeV1::ScalarString,
                ..
            },
            Result::Scalar { value },
            None,
        ) if sql_value_size_valid(value) => Ok(()),
        (
            SqlServiceContinuation::Execute {
                mode: era_runtime_protocol::SqlExecuteModeV1::Reader,
                ..
            },
            Result::ReaderOpened { reader },
            Some(state),
        ) if *reader == state.reader
            && reader.service_epoch == response.provider.service_epoch
            && state.status == Status::BeforeFirst
            && state.rows_read == 0 =>
        {
            Ok(())
        }
        (
            SqlServiceContinuation::ReaderRead { .. },
            Result::ReaderAdvanced { has_row },
            Some(state),
        ) if if *has_row {
            state.status == Status::Row
                && current_reader.is_some_and(|current| {
                    current.rows_read.checked_add(1) == Some(state.rows_read)
                })
        } else {
            state.status == Status::Eof
                && current_reader.is_some_and(|current| state.rows_read == current.rows_read)
        } =>
        {
            Ok(())
        }
        (SqlServiceContinuation::ReaderGet { .. }, Result::ReaderValue { value }, Some(state))
            if state.status == Status::Row
                && current_reader.is_some_and(|current| state.rows_read == current.rows_read)
                && sql_value_size_valid(value) =>
        {
            Ok(())
        }
        (SqlServiceContinuation::ReaderIsNull { .. }, Result::ReaderNull { .. }, Some(state))
            if state.status == Status::Row
                && current_reader.is_some_and(|current| state.rows_read == current.rows_read) =>
        {
            Ok(())
        }
        (SqlServiceContinuation::ReaderClose { .. }, Result::ReaderClosed, Some(state))
            if state.status == Status::Closed
                && current_reader.is_some_and(|current| state.rows_read == current.rows_read) =>
        {
            Ok(())
        }
        (
            SqlServiceContinuation::ImportMap { expected_rows, .. },
            Result::MapImported { rows },
            None,
        ) if rows == expected_rows => Ok(()),
        (SqlServiceContinuation::Disconnect { .. }, Result::Disconnected, None) => Ok(()),
        _ => Err("SQL response result or state shape differs from its continuation"),
    }
}

fn sql_value_size_valid(value: &era_runtime_protocol::SqlValueV1) -> bool {
    match value {
        era_runtime_protocol::SqlValueV1::String(value) => {
            value.len() <= era_runtime_protocol::SqlLimitsV1::FIXED.maximum_cell_bytes as usize
        }
        era_runtime_protocol::SqlValueV1::Null | era_runtime_protocol::SqlValueV1::Integer(_) => {
            true
        }
    }
}
