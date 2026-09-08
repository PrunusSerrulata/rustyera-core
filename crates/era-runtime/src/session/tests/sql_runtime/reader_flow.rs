#[test]
fn reusable_scalar_results_skip_provider_round_trips_until_a_write() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        SQL_CONNECT \"other\"\n\
        RESULT:0 = SQL_EXECUTE_SCALAR_LONG(\"db\", \"SELECT value FROM data\")\n\
        RESULT:1 = SQL_EXECUTE_SCALAR_LONG(\"db\", \"SELECT value FROM data\")\n\
        RESULT:2 = SQL_EXECUTE_NONQUERY(\"other\", \"UPDATE data SET value = 8\")\n\
        RESULT:3 = SQL_EXECUTE_SCALAR_LONG(\"db\", \"SELECT value FROM data\")\n\
        WAIT\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    let other_open = take_sql_request(messages);
    let messages = harness.respond(&other_open, open_response(&other_open, revision(2)));
    let first_scalar = take_sql_request(messages);
    assert!(matches!(
        &first_scalar.payload.operation,
        SqlOperationV1::Execute { sql, .. } if sql == "SELECT value FROM data"
    ));

    let messages = harness.respond(
        &first_scalar,
        execute_response(
            &first_scalar,
            false,
            revision(3),
            SqlResultV1::ReusableScalar {
                value: SqlValueV1::Integer(7),
            },
        ),
    );
    let write = take_sql_request(messages);
    assert_eq!(harness.integer(0), 7);
    assert_eq!(harness.integer(1), 7);
    assert!(matches!(
        &write.payload.operation,
        SqlOperationV1::Execute { sql, .. } if sql == "UPDATE data SET value = 8"
    ));

    let messages = harness.respond(
        &write,
        execute_response(
            &write,
            false,
            revision(4),
            SqlResultV1::NonQuery { affected_rows: 1 },
        ),
    );
    let scalar_after_write = take_sql_request(messages);
    assert!(matches!(
        &scalar_after_write.payload.operation,
        SqlOperationV1::Execute { sql, .. } if sql == "SELECT value FROM data"
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn real_vm_accepts_out_of_order_completions_across_connections() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"alpha\"\n\
        SQL_CONNECT \"beta\"\n\
        IF 0\n\
        CALL EXEC_ALPHA\n\
        CALL EXEC_BETA\n\
        ENDIF\n\
        WAIT\n\
        RETURN\n\
        @EXEC_ALPHA\n\
        #LOCALSIZE 1\n\
        LOCAL = SQL_EXECUTE_NONQUERY(\"alpha\", \"UPDATE alpha\")\n\
        RESULT:0 = LOCAL\n\
        RETURN\n\
        @EXEC_BETA\n\
        #LOCALSIZE 1\n\
        LOCAL = SQL_EXECUTE_NONQUERY(\"beta\", \"UPDATE beta\")\n\
        RESULT:1 = LOCAL\n\
        RETURN\n";
    let (mut harness, alpha_open) = SqlHarness::start(source);
    let SqlOperationV1::Open {
        logical_name: alpha_name,
        ..
    } = &alpha_open.payload.operation
    else {
        panic!("first request is alpha Open")
    };
    assert_eq!(alpha_name, "alpha");
    let alpha_handle = operation_connection(&alpha_open);

    let messages = harness.respond(&alpha_open, open_response(&alpha_open, revision(1)));
    let beta_open = take_sql_request(messages);
    let SqlOperationV1::Open {
        logical_name: beta_name,
        ..
    } = &beta_open.payload.operation
    else {
        panic!("second request is beta Open")
    };
    assert_eq!(beta_name, "beta");
    let beta_handle = operation_connection(&beta_open);

    let messages = harness.respond(&beta_open, open_response(&beta_open, revision(2)));
    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);

    let fibers = spawn_entries(&mut harness, &["EXEC_ALPHA", "EXEC_BETA"]);
    assert!(fibers.iter().all(|fiber| matches!(
        harness
            .session
            .vm
            .as_ref()
            .expect("runtime VM")
            .fiber_status(*fiber),
        Some(erabasic_vm::FiberStatus::WaitingHost(_))
    )));
    let mut executes = take_sql_requests(harness.drive_to_boundary());
    assert_eq!(
        executes.len(),
        2,
        "both fibers must issue SQL before either completes"
    );
    let beta_index = executes
        .iter()
        .position(|request| {
            matches!(
                &request.payload.operation,
                SqlOperationV1::Execute { connection, sql, .. }
                    if *connection == beta_handle && sql == "UPDATE beta"
            )
        })
        .expect("beta execute request");
    let beta_execute = executes.remove(beta_index);
    let alpha_execute = executes.pop().expect("alpha execute request");
    assert!(matches!(
        &alpha_execute.payload.operation,
        SqlOperationV1::Execute { connection, sql, .. }
            if *connection == alpha_handle && sql == "UPDATE alpha"
    ));
    let messages = harness.respond(
        &beta_execute,
        execute_response(
            &beta_execute,
            false,
            revision(12),
            SqlResultV1::NonQuery { affected_rows: 2 },
        ),
    );
    assert_no_sql_request(&messages);
    assert_eq!(
        harness
            .session
            .sql
            .connection_by_key("alpha")
            .and_then(|connection| connection.durable_revision.as_ref()),
        Some(&revision(1)),
        "alpha completion is still pending"
    );
    let vm = harness.session.vm.as_ref().expect("runtime VM");
    assert!(matches!(
        vm.fiber_status(fibers[0]),
        Some(erabasic_vm::FiberStatus::WaitingHost(_))
    ));
    assert_eq!(vm.fiber_status(fibers[1]), None);
    let messages = harness.respond(
        &alpha_execute,
        execute_response(
            &alpha_execute,
            false,
            revision(11),
            SqlResultV1::NonQuery { affected_rows: 1 },
        ),
    );

    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::Running);
    let vm = harness.session.vm.as_ref().expect("runtime VM");
    assert_eq!(vm.fiber_status(fibers[0]), None);
    assert_eq!(vm.fiber_status(fibers[1]), None);
    assert_eq!(
        harness
            .session
            .sql
            .connection_by_key("alpha")
            .and_then(|connection| connection.durable_revision.as_ref()),
        Some(&revision(11))
    );
    assert_eq!(
        harness
            .session
            .sql
            .connection_by_key("beta")
            .and_then(|connection| connection.durable_revision.as_ref()),
        Some(&revision(12))
    );
}

#[test]
fn real_vm_reader_get_eof_and_close_follow_provider_state() {
    reader_get_eof_and_close(false);
}

#[test]
fn projected_reader_row_skips_column_round_trips_and_is_retired_at_eof() {
    reader_get_eof_and_close(true);
}

#[test]
fn projected_reader_row_rejects_excessive_columns_or_bytes() {
    use era_runtime_protocol::SqlReaderCellV1;
    for cells in [
        vec![
            SqlReaderCellV1 {
                integer: None,
                string: None,
                is_null: None
            };
            33
        ],
        vec![SqlReaderCellV1 {
            integer: None,
            string: Some("x".repeat(65521)),
            is_null: None,
        }],
    ] {
        let (mut harness, request, connection, reader) = projected_reader_harness();
        harness.respond(
            &request,
            reader_response(
                &request,
                connection,
                revision(1),
                reader,
                SqlReaderStatusV1::Row,
                1,
                SqlResultV1::ReaderRow { cells },
            ),
        );
        assert_eq!(harness.session.phase(), RuntimePhase::Faulted);
    }
}

#[test]
fn projected_reader_row_uses_fallback_for_missing_conversion() {
    let (mut harness, request, connection, reader) = projected_reader_harness();
    let messages = harness.respond(
        &request,
        reader_response(
            &request,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            1,
            SqlResultV1::ReaderRow {
                cells: vec![era_runtime_protocol::SqlReaderCellV1 {
                    integer: None,
                    string: Some("text".into()),
                    is_null: Some(false),
                }],
            },
        ),
    );
    let get = take_sql_request(messages);
    assert!(matches!(
        get.payload.operation,
        SqlOperationV1::ReaderGet { column: 0, .. }
    ));
    assert_eq!(harness.integer(0), 1);
}

#[test]
fn projected_reader_accepts_exact_utf8_budget_and_preserves_outside_column_errors() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nLOCAL = SQL_EXECUTE_READER(\"db\", \"SELECT value\")\nRESULT:0 = SQL_READER_READ(LOCAL)\nRESULT:1 = SQL_READER_GET_LONG(LOCAL, 32)\nWAIT\n";
    let (mut harness, read, connection, reader) =
        projected_reader_source(source, era_runtime_protocol::SQL_READER_ROW_VERSION);
    let mut cells = vec![
        era_runtime_protocol::SqlReaderCellV1 {
            integer: None,
            string: None,
            is_null: None
        };
        32
    ];
    cells[0].string = Some("界".repeat(21674) + "xx");
    assert_eq!(cells[0].string.as_ref().unwrap().len() + 32 * 16, 65536);
    let messages = harness.respond(
        &read,
        reader_response(
            &read,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            1,
            SqlResultV1::ReaderRow { cells },
        ),
    );
    let get = take_sql_request(messages);
    assert!(matches!(
        get.payload.operation,
        SqlOperationV1::ReaderGet { column: 32, .. }
    ));
    harness.respond(
        &get,
        reader_response(
            &get,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            1,
            SqlResultV1::Error {
                error: SqlErrorV1 {
                    code: SqlErrorCodeV1::ColumnOutOfRange,
                    operation: SqlOperationKindV1::ReaderGet,
                    context: vec![],
                    sqlite_code: None,
                    sqlite_message: None,
                },
            },
        ),
    );
    assert_eq!(harness.session.phase(), RuntimePhase::Faulted);
}

fn projected_reader_harness() -> (
    SqlHarness,
    CapturedSqlRequest,
    SqlConnectionHandleV1,
    SqlReaderHandleV1,
) {
    projected_reader_harness_version(era_runtime_protocol::SQL_READER_ROW_VERSION)
}

fn projected_reader_harness_version(
    version: ProtocolVersion,
) -> (
    SqlHarness,
    CapturedSqlRequest,
    SqlConnectionHandleV1,
    SqlReaderHandleV1,
) {
    projected_reader_source(
        "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nLOCAL = SQL_EXECUTE_READER(\"db\", \"SELECT value\")\nRESULT:0 = SQL_READER_READ(LOCAL)\nRESULT:1 = SQL_READER_GET_LONG(LOCAL, 0)\nWAIT\n",
        version,
    )
}

fn projected_reader_source(
    source: &str,
    version: ProtocolVersion,
) -> (
    SqlHarness,
    CapturedSqlRequest,
    SqlConnectionHandleV1,
    SqlReaderHandleV1,
) {
    let (mut harness, open) = SqlHarness::start_version(source, version);
    let connection = operation_connection(&open);
    let execute = take_sql_request_version(
        harness.respond(&open, open_response(&open, revision(1))),
        version,
    );
    let reader = SqlReaderHandleV1 {
        service_epoch: execute.payload.provider.service_epoch,
        id: 71,
    };
    let messages = harness.respond(
        &execute,
        reader_response(
            &execute,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::BeforeFirst,
            0,
            SqlResultV1::ReaderOpened { reader },
        ),
    );
    (
        harness,
        take_sql_request_version(messages, version),
        connection,
        reader,
    )
}

#[test]
fn projected_reader_replaces_rows_and_keeps_read_inflight_uncached() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nLOCAL = SQL_EXECUTE_READER(\"db\", \"SELECT value\")\nRESULT:0 = SQL_READER_READ(LOCAL)\nRESULT:1 = SQL_READER_GET_LONG(LOCAL, 0)\nRESULT:2 = SQL_READER_READ(LOCAL)\nRESULT:3 = SQL_READER_GET_LONG(LOCAL, 0)\nRESULT:4 = SQL_READER_ISNULL(LOCAL, 0)\nRESULTS '= SQL_READER_GET_STRING(LOCAL, 0)\nWAIT\n";
    let (mut harness, first, connection, reader) =
        projected_reader_source(source, era_runtime_protocol::SQL_READER_ROW_VERSION);
    let messages = harness.respond(
        &first,
        reader_response(
            &first,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            1,
            SqlResultV1::ReaderRow {
                cells: vec![era_runtime_protocol::SqlReaderCellV1 {
                    integer: Some(17),
                    string: Some("17".into()),
                    is_null: Some(false),
                }],
            },
        ),
    );
    let second = take_sql_request(messages);
    assert!(matches!(
        second.payload.operation,
        SqlOperationV1::ReaderRead { .. }
    ));
    assert_eq!(harness.integer(1), 17);
    assert!(harness.session.sql.reader(1).unwrap().row.is_empty());
    assert!(!harness.session.sql.reserve_connection("db"));
    let messages = harness.respond(
        &second,
        reader_response(
            &second,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            2,
            SqlResultV1::ReaderRow {
                cells: vec![era_runtime_protocol::SqlReaderCellV1 {
                    integer: Some(0),
                    string: Some(String::new()),
                    is_null: Some(true),
                }],
            },
        ),
    );
    assert_no_sql_request(&messages);
    assert_eq!(harness.integer(3), 0);
    assert_eq!(harness.integer(4), 1);
    assert_eq!(
        read_runtime_string(harness.session.vm.as_ref().unwrap(), "RESULTS").unwrap(),
        ""
    );
}

#[test]
fn projected_reader_fallback_error_retires_old_row() {
    let (mut harness, read, connection, reader) = projected_reader_harness();
    let messages = harness.respond(
        &read,
        reader_response(
            &read,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            1,
            SqlResultV1::ReaderRow {
                cells: vec![era_runtime_protocol::SqlReaderCellV1 {
                    integer: None,
                    string: Some("old".into()),
                    is_null: Some(false),
                }],
            },
        ),
    );
    let get = take_sql_request(messages);
    harness.respond(
        &get,
        reader_response(
            &get,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            2,
            SqlResultV1::Error {
                error: SqlErrorV1 {
                    code: SqlErrorCodeV1::TypeMismatch,
                    operation: SqlOperationKindV1::ReaderGet,
                    context: vec![],
                    sqlite_code: None,
                    sqlite_message: None,
                },
            },
        ),
    );
    assert!(
        harness
            .session
            .sql
            .reader(1)
            .is_none_or(|state| state.row.is_empty())
    );
}

#[test]
fn reader_row_extension_negotiates_down_to_both_older_versions() {
    for version in [ProtocolVersion::new(1, 0), ProtocolVersion::new(1, 1)] {
        let (mut harness, read, connection, reader) = projected_reader_harness_version(version);
        let messages = harness.respond(
            &read,
            reader_response(
                &read,
                connection,
                revision(1),
                reader,
                SqlReaderStatusV1::Row,
                1,
                SqlResultV1::ReaderAdvanced { has_row: true },
            ),
        );
        let get = take_sql_request_version(messages, version);
        assert!(matches!(
            get.payload.operation,
            SqlOperationV1::ReaderGet { .. }
        ));
        let messages = harness.respond(
            &get,
            reader_response(
                &get,
                connection,
                revision(1),
                reader,
                SqlReaderStatusV1::Row,
                1,
                SqlResultV1::ReaderValue {
                    value: SqlValueV1::Integer(9),
                },
            ),
        );
        assert_no_sql_request(&messages);
        assert_eq!(harness.integer(1), 9);
        assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);

        let (mut harness, read, connection, reader) = projected_reader_harness_version(version);
        harness.respond(
            &read,
            reader_response(
                &read,
                connection,
                revision(1),
                reader,
                SqlReaderStatusV1::Row,
                1,
                SqlResultV1::ReaderRow { cells: vec![] },
            ),
        );
        assert_eq!(harness.session.phase(), RuntimePhase::Faulted);
    }
}

#[allow(clippy::too_many_lines)]
fn reader_get_eof_and_close(project_row: bool) {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        LOCAL = SQL_EXECUTE_READER(\"db\", \"SELECT value\")\n\
        RESULT:0 = SQL_READER_READ(LOCAL)\n\
        RESULT:1 = SQL_READER_GET_LONG(LOCAL, 0)\n\
        RESULTS:0 '= SQL_READER_GET_STRING(LOCAL, 0)\n\
        RESULT:2 = SQL_READER_READ(LOCAL)\n\
        RESULT:3 = SQL_READER_READ(LOCAL)\n\
        SQL_READER_CLOSE LOCAL\n\
        SQL_READER_CLOSE LOCAL\n\
        RESULT:4 = 1\n\
        WAIT\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let connection = operation_connection(&open);
    let messages = harness.respond(&open, open_response(&open, revision(1)));

    let execute = take_sql_request(messages);
    assert!(matches!(
        &execute.payload.operation,
        SqlOperationV1::Execute {
            mode: era_runtime_protocol::SqlExecuteModeV1::Reader,
            ..
        }
    ));
    let reader = SqlReaderHandleV1 {
        service_epoch: execute.payload.provider.service_epoch,
        id: 71,
    };
    let messages = harness.respond(
        &execute,
        reader_response(
            &execute,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::BeforeFirst,
            0,
            SqlResultV1::ReaderOpened { reader },
        ),
    );

    let read_row = take_sql_request(messages);
    assert!(matches!(
        &read_row.payload.operation,
        SqlOperationV1::ReaderRead { reader: value } if *value == reader
    ));
    let messages = harness.respond(
        &read_row,
        reader_response(
            &read_row,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Row,
            1,
            if project_row {
                SqlResultV1::ReaderRow {
                    cells: vec![era_runtime_protocol::SqlReaderCellV1 {
                        integer: Some(42),
                        string: Some("42".into()),
                        is_null: Some(false),
                    }],
                }
            } else {
                SqlResultV1::ReaderAdvanced { has_row: true }
            },
        ),
    );

    let messages = if project_row {
        messages
    } else {
        let get = take_sql_request(messages);
        assert!(matches!(
            &get.payload.operation,
            SqlOperationV1::ReaderGet {
                reader: value,
                column: 0,
                mode: era_runtime_protocol::SqlReaderValueModeV1::Integer,
            } if *value == reader
        ));
        let messages = harness.respond(
            &get,
            reader_response(
                &get,
                connection,
                revision(1),
                reader,
                SqlReaderStatusV1::Row,
                1,
                SqlResultV1::ReaderValue {
                    value: SqlValueV1::Integer(42),
                },
            ),
        );

        let get_string = take_sql_request(messages);
        assert!(matches!(
            &get_string.payload.operation,
            SqlOperationV1::ReaderGet {
                reader: value,
                column: 0,
                mode: era_runtime_protocol::SqlReaderValueModeV1::String,
            } if *value == reader
        ));

        harness.respond(
            &get_string,
            reader_response(
                &get_string,
                connection,
                revision(1),
                reader,
                SqlReaderStatusV1::Row,
                1,
                SqlResultV1::ReaderValue {
                    value: SqlValueV1::String("42".into()),
                },
            ),
        )
    };
    let read_eof = take_sql_request(messages);
    assert!(matches!(
        &read_eof.payload.operation,
        SqlOperationV1::ReaderRead { .. }
    ));
    let messages = harness.respond(
        &read_eof,
        reader_response(
            &read_eof,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Eof,
            1,
            SqlResultV1::ReaderAdvanced { has_row: false },
        ),
    );
    assert!(
        harness
            .session
            .sql
            .reader(1)
            .is_none_or(|reader| reader.row.is_empty())
    );
    let close = take_sql_request(messages);
    assert!(matches!(
        &close.payload.operation,
        SqlOperationV1::ReaderClose { reader: value } if *value == reader
    ));
    let messages = harness.respond(
        &close,
        reader_response(
            &close,
            connection,
            revision(1),
            reader,
            SqlReaderStatusV1::Closed,
            1,
            SqlResultV1::ReaderClosed,
        ),
    );

    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(harness.integer(0), 1);
    assert_eq!(harness.integer(1), 42);
    assert_eq!(
        read_runtime_string(harness.session.vm.as_ref().unwrap(), "RESULTS").unwrap(),
        "42"
    );
    assert_eq!(harness.integer(2), 0);
    assert_eq!(harness.integer(3), 0);
    assert_eq!(harness.integer(4), 1);
    assert!(!harness.session.sql.has_active_readers());
}
