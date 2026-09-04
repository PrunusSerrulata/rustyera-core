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
