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
#[allow(clippy::too_many_lines)]
fn real_vm_reader_get_eof_and_close_follow_provider_state() {
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
            SqlResultV1::ReaderAdvanced { has_row: true },
        ),
    );

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
    let messages = harness.respond(
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
    );

    let read_eof = take_sql_request(messages);
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
    assert_eq!(harness.integer(2), 0);
    assert_eq!(harness.integer(3), 0);
    assert_eq!(harness.integer(4), 1);
    assert!(!harness.session.sql.has_active_readers());
}
