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
