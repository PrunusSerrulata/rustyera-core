use super::*;

use era_runtime_protocol::{
    SqlConnectionHandleV1, SqlDatabaseStateV1, SqlErrorCodeV1, SqlErrorContextV1, SqlErrorV1,
    SqlOperationKindV1, SqlOperationV1, SqlReaderHandleV1, SqlReaderStateV1, SqlReaderStatusV1,
    SqlRequestV1, SqlResponseV1, SqlResultV1, SqlRevisionV1, SqlValueV1,
};

struct CapturedSqlRequest {
    wire: ServiceRequest,
    payload: SqlRequestV1,
}

struct SqlHarness {
    session: RuntimeSession,
    next_sequence: u64,
}

impl SqlHarness {
    fn start(source: &str) -> (Self, CapturedSqlRequest) {
        let profile = erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
        let mut session = negotiated_session();
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                compatibility: era_runtime_protocol::CompatibilityIdentity::for_profile(profile),
                project_revision: 1,
                files: vec![
                    profile_configuration_file(profile),
                    SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(source.into()),
                        content_hash: None,
                    },
                ],
            }),
        );
        session
            .drive(RuntimeDriveBudget::default())
            .expect("load the SQL integration fixture");
        let loaded = drain(&mut session);
        assert_eq!(session.phase(), RuntimePhase::Ready, "{loaded:#?}");
        assert!(loaded.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(report) if report.success
        )));
        assert!(session.vm.is_none());

        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        let mut harness = Self {
            session,
            next_sequence: 3,
        };
        let messages = harness.drive_to_boundary();
        assert!(
            harness.session.vm.is_some(),
            "the script created a RuntimeVm"
        );
        let request = take_sql_request(messages);
        (harness, request)
    }

    fn drive_to_boundary(&mut self) -> Vec<RuntimeMessage> {
        let mut messages = Vec::new();
        for _ in 0..16 {
            self.session
                .drive(RuntimeDriveBudget::default())
                .expect("drive the SQL fixture");
            let emitted = drain(&mut self.session);
            let has_sql_request = emitted.iter().any(|message| {
                matches!(
                    message,
                    RuntimeMessage::ServiceRequest(request)
                        if request.kind == ServiceKind::Sql && request.operation == SQL_OPERATION
                )
            });
            messages.extend(emitted);
            if has_sql_request
                || matches!(
                    self.session.phase(),
                    RuntimePhase::WaitingExternal
                        | RuntimePhase::WaitingInput
                        | RuntimePhase::Faulted
                        | RuntimePhase::Stopped
                )
            {
                break;
            }
        }
        messages
    }

    fn respond(
        &mut self,
        request: &CapturedSqlRequest,
        response: SqlResponseV1,
    ) -> Vec<RuntimeMessage> {
        self.respond_to_request_id(request.wire.request_id, response)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn respond_to_request_id(
        &mut self,
        request_id: u64,
        response: SqlResponseV1,
    ) -> Vec<RuntimeMessage> {
        let payload = era_protocol::encode_canonical(&response)
            .expect("encode the fake SQL provider response");
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("test sequence remains bounded");
        submit(
            &mut self.session,
            sequence,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(payload),
                },
            }),
        );
        self.drive_to_boundary()
    }

    fn submit_message(&mut self, message: RuntimeMessage) -> Vec<RuntimeMessage> {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("test sequence remains bounded");
        submit(&mut self.session, sequence, message);
        self.drive_to_boundary()
    }

    fn integer(&self, index: u64) -> i64 {
        read_runtime_integer(
            self.session.vm.as_ref().expect("runtime VM"),
            "RESULT",
            &[index],
            None,
        )
        .expect("read SQL fixture result")
    }
}

fn take_sql_request(messages: Vec<RuntimeMessage>) -> CapturedSqlRequest {
    let mut requests = take_sql_requests(messages);
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one outbound SQL request"
    );
    requests.pop().expect("one SQL request")
}

fn take_sql_requests(messages: Vec<RuntimeMessage>) -> Vec<CapturedSqlRequest> {
    messages
        .into_iter()
        .filter_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::Sql && request.operation == SQL_OPERATION =>
            {
                let payload = era_protocol::decode_canonical(request.payload.as_slice())
                    .expect("decode SqlRequestV1 from the runtime envelope");
                assert_eq!(request.operation_version, SQL_OPERATION_VERSION);
                Some(CapturedSqlRequest {
                    wire: request,
                    payload,
                })
            }
            _ => None,
        })
        .collect()
}

fn assert_no_sql_request(messages: &[RuntimeMessage]) {
    assert!(messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.kind == ServiceKind::Sql && request.operation == SQL_OPERATION
    )));
}

fn spawn_entries(harness: &mut SqlHarness, names: &[&str]) -> Vec<erabasic_vm::FiberId> {
    let artifact = harness
        .session
        .artifact
        .clone()
        .expect("compiled SQL fixture artifact");
    let mut replacement = RuntimeVm::new(artifact, VmConfig::default());
    let entries = {
        let artifact = replacement.vm().artifact();
        names
            .iter()
            .map(|name| {
                artifact
                    .functions
                    .iter()
                    .find(|function| function.name == *name)
                    .unwrap_or_else(|| panic!("missing {name} entry"))
                    .key
            })
            .collect::<Vec<_>>()
    };
    let fibers = names
        .iter()
        .zip(entries)
        .map(|(name, entry)| {
            replacement
                .spawn_entry(entry, Vec::new())
                .unwrap_or_else(|error| panic!("spawn {name} SQL fiber: {error}"))
        })
        .collect::<Vec<_>>();
    harness
        .session
        .operations
        .take_active_input()
        .expect("replace the stable fixture wait");
    harness.session.active_input_source = None;
    harness.session.controller.clear();
    harness.session.controller.flow = None;
    harness.session.phase = RuntimePhase::Running;
    let report = replacement.drive(RunBudget::default(), VmDriveMode::Normal);
    assert!(
        !report.events.is_empty(),
        "spawned SQL fibers must produce VM events"
    );
    for event in report.events {
        harness
            .session
            .handle_vm_event(&mut replacement, event)
            .expect("route spawned SQL VM event through RuntimeSession");
    }
    harness.session.vm = Some(replacement);
    fibers
}

fn revision(byte: u8) -> SqlRevisionV1 {
    SqlRevisionV1 {
        sha256: ProtocolBytes::new(vec![byte; 32]),
    }
}

fn operation_connection(request: &CapturedSqlRequest) -> SqlConnectionHandleV1 {
    match &request.payload.operation {
        SqlOperationV1::Open { connection, .. }
        | SqlOperationV1::Execute { connection, .. }
        | SqlOperationV1::ImportMapRows { connection, .. }
        | SqlOperationV1::Disconnect { connection } => *connection,
        SqlOperationV1::ReaderRead { .. }
        | SqlOperationV1::ReaderGet { .. }
        | SqlOperationV1::ReaderIsNull { .. }
        | SqlOperationV1::ReaderClose { .. } => {
            panic!("reader operation needs its owning connection from the fixture")
        }
    }
}

fn database_state(
    connection: SqlConnectionHandleV1,
    connected: bool,
    transaction_active: bool,
    durable_revision: SqlRevisionV1,
) -> SqlDatabaseStateV1 {
    SqlDatabaseStateV1 {
        connection,
        connected,
        transaction_active,
        durable_revision: Some(durable_revision),
    }
}

fn open_response(request: &CapturedSqlRequest, durable_revision: SqlRevisionV1) -> SqlResponseV1 {
    let SqlOperationV1::Open {
        connection,
        logical_name,
        identity,
        limits,
        ..
    } = &request.payload.operation
    else {
        panic!("expected SQL Open request")
    };
    assert!(!logical_name.is_empty());
    assert_eq!(
        identity.sqlite_version,
        era_runtime_protocol::SQL_SQLITE_VERSION
    );
    assert_eq!(*limits, era_runtime_protocol::SqlLimitsV1::FIXED);
    SqlResponseV1 {
        provider: request.payload.provider,
        database: Some(database_state(*connection, true, false, durable_revision)),
        reader: None,
        result: SqlResultV1::Opened {
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            limits: era_runtime_protocol::SqlLimitsV1::FIXED,
        },
    }
}

fn execute_response(
    request: &CapturedSqlRequest,
    transaction_active: bool,
    durable_revision: SqlRevisionV1,
    result: SqlResultV1,
) -> SqlResponseV1 {
    SqlResponseV1 {
        provider: request.payload.provider,
        database: Some(database_state(
            operation_connection(request),
            true,
            transaction_active,
            durable_revision,
        )),
        reader: None,
        result,
    }
}

fn reader_response(
    request: &CapturedSqlRequest,
    connection: SqlConnectionHandleV1,
    durable_revision: SqlRevisionV1,
    reader: SqlReaderHandleV1,
    status: SqlReaderStatusV1,
    rows_read: u64,
    result: SqlResultV1,
) -> SqlResponseV1 {
    SqlResponseV1 {
        provider: request.payload.provider,
        database: Some(database_state(connection, true, false, durable_revision)),
        reader: Some(SqlReaderStateV1 {
            reader,
            status,
            rows_read,
        }),
        result,
    }
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

#[test]
fn structured_sql_error_applies_authoritative_state_before_vm_fault() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        SQL_EXECUTE_NONQUERY \"db\", \"BEGIN broken\"\n\
        WAIT\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    let execute = take_sql_request(messages);
    let response = execute_response(
        &execute,
        true,
        revision(2),
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::Sqlite,
                operation: SqlOperationKindV1::Execute,
                context: vec![SqlErrorContextV1 {
                    key: "statement".into(),
                    value: "begin".into(),
                }],
                sqlite_code: Some(1),
                sqlite_message: Some("diagnostic text is not classification".into()),
            },
        },
    );
    let messages = harness.respond(&execute, response);

    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    let connection = harness
        .session
        .sql
        .connection_by_key("db")
        .expect("SQL error keeps the authoritative connection");
    assert!(connection.transaction_active);
    assert_eq!(connection.durable_revision.as_ref(), Some(&revision(2)));
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn storage_failure_keeps_the_last_provider_revision_and_faults_the_vm() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        SQL_EXECUTE_NONQUERY \"db\", \"UPDATE durable\"\n\
        WAIT\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    let execute = take_sql_request(messages);
    let response = execute_response(
        &execute,
        false,
        revision(1),
        SqlResultV1::Error {
            error: SqlErrorV1 {
                code: SqlErrorCodeV1::StorageFailure,
                operation: SqlOperationKindV1::Execute,
                context: vec![SqlErrorContextV1 {
                    key: "stage".into(),
                    value: "persist".into(),
                }],
                sqlite_code: None,
                sqlite_message: Some("provider-local diagnostic".into()),
            },
        },
    );
    let messages = harness.respond(&execute, response);

    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault { code: FaultCode::VmFault, message, .. })
            if message.contains("rustyera.sql/execute/storage_failure")
                && !message.contains("provider-local diagnostic")
    )));
    let connection = harness
        .session
        .sql
        .connection_by_key("db")
        .expect("storage failure retains the authoritative connection");
    assert_eq!(connection.durable_revision.as_ref(), Some(&revision(1)));
    assert!(!connection.transaction_active);
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn concurrent_same_name_open_has_one_reservation_and_one_provider_request() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"setup\"\n\
        IF 0\n\
        CALL OPEN_FIRST\n\
        CALL OPEN_SECOND\n\
        ENDIF\n\
        WAIT\n\
        RETURN\n\
        @OPEN_FIRST\n\
        SQL_CONNECT \"race\"\n\
        RETURN\n\
        @OPEN_SECOND\n\
        SQL_CONNECT \"RACE\"\n\
        RETURN\n";
    let (mut harness, setup_open) = SqlHarness::start(source);
    let messages = harness.respond(&setup_open, open_response(&setup_open, revision(1)));
    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);

    drop(spawn_entries(&mut harness, &["OPEN_FIRST", "OPEN_SECOND"]));
    let messages = harness.drive_to_boundary();
    let requests = take_sql_requests(messages);
    assert_eq!(
        requests.len(),
        1,
        "only the reserved Open reaches the provider"
    );
    assert!(matches!(
        &requests[0].payload.operation,
        SqlOperationV1::Open { logical_name, .. } if logical_name == "race"
    ));
    assert!(harness.session.sql.has_inflight());
    assert!(harness.session.sql.connection_by_key("race").is_none());

    let messages = harness.respond(&requests[0], open_response(&requests[0], revision(2)));
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    assert!(harness.session.sql.connection_by_key("race").is_some());
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn disconnect_during_execute_is_rejected_without_reaching_the_provider() {
    let source = "@SYSTEM_TITLE\n\
        SQL_CONNECT \"db\"\n\
        IF 0\n\
        CALL EXECUTE_PENDING\n\
        CALL DISCONNECT_PENDING\n\
        ENDIF\n\
        WAIT\n\
        RETURN\n\
        @EXECUTE_PENDING\n\
        SQL_EXECUTE_NONQUERY \"db\", \"UPDATE pending\"\n\
        RETURN\n\
        @DISCONNECT_PENDING\n\
        SQL_DISCONNECT \"db\"\n\
        RETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let messages = harness.respond(&open, open_response(&open, revision(1)));
    assert_no_sql_request(&messages);
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingInput);

    drop(spawn_entries(
        &mut harness,
        &["EXECUTE_PENDING", "DISCONNECT_PENDING"],
    ));
    let messages = harness.drive_to_boundary();
    let requests = take_sql_requests(messages);
    assert_eq!(
        requests.len(),
        1,
        "disconnect must remain local while Execute is pending"
    );
    assert!(matches!(
        &requests[0].payload.operation,
        SqlOperationV1::Execute { sql, .. } if sql == "UPDATE pending"
    ));
    assert!(harness.session.sql.has_inflight());
    assert!(harness.session.sql.connection_by_key("db").is_some());

    let messages = harness.respond(
        &requests[0],
        execute_response(
            &requests[0],
            false,
            revision(2),
            SqlResultV1::NonQuery { affected_rows: 1 },
        ),
    );
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    assert_no_sql_request(&messages);
    assert!(!harness.session.sql.has_inflight());
}

#[test]
fn stale_provider_response_is_rejected_and_the_real_request_can_retry() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nRESULT:0 = 1\nWAIT\nRETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let mut stale = open_response(&open, revision(1));
    stale.provider.id = stale
        .provider
        .id
        .checked_add(1)
        .expect("provider id remains bounded");
    let messages = harness.respond(&open, stale);

    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::StaleRequest,
            ..
        })
    )));
    assert_eq!(harness.session.phase(), RuntimePhase::WaitingExternal);
    assert_no_sql_request(&messages);

    let messages = harness.respond(&open, open_response(&open, revision(1)));
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::WaitingInput,
        "{messages:#?}"
    );
    assert_eq!(harness.integer(0), 1);
    assert!(harness.session.sql.connection_by_key("db").is_some());
}

#[test]
fn project_boundary_cancels_the_request_and_rejects_its_old_epoch_completion() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nWAIT\nRETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let old_epoch = open.payload.provider.service_epoch;
    let old_response = open_response(&open, revision(1));
    let shutdown = harness.submit_message(RuntimeMessage::ShutdownRequest(ShutdownRequest {
        graceful: true,
    }));
    assert!(shutdown.iter().any(|message| matches!(
        message,
        RuntimeMessage::CancelExternalRequest(cancel)
            if cancel.request_id == open.wire.request_id
    )));
    assert!(
        shutdown
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ShutdownReady(_)))
    );
    assert_eq!(harness.session.phase(), RuntimePhase::Stopped);
    assert_ne!(harness.session.sql.service_epoch(), old_epoch);

    let messages = harness.respond_to_request_id(open.wire.request_id, old_response);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::StaleRequest,
            ..
        })
    )));
    assert!(harness.session.sql.connection_by_key("db").is_none());
}

#[test]
fn snake_project_without_sql_capability_never_creates_a_vm() {
    let profile = erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
    let mut session = negotiated_session_without_sql();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::for_profile(profile),
            project_revision: 1,
            files: vec![
                profile_configuration_file(profile),
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nRESULT = SQL_CONNECT(\"db\")\nRETURN\n".into(),
                    ),
                    content_hash: None,
                },
            ],
        }),
    );
    session
        .drive(RuntimeDriveBudget::default())
        .expect("reject the snake fixture before compilation");
    let messages = drain(&mut session);

    assert!(session.vm.is_none());
    assert!(session.project_snapshot.is_none());
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report)
            if !report.success && report.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "runtime.missing_sql_service"
            })
    )));
    assert_no_sql_request(&messages);
}

#[test]
fn ninth_connection_is_rejected_by_the_connection_and_opening_limit() {
    let mut source = String::from("@SYSTEM_TITLE\n");
    for index in 0..9 {
        source.push_str("RESULT:");
        source.push_str(&index.to_string());
        source.push_str(" = SQL_CONNECT(\"db");
        source.push_str(&index.to_string());
        source.push_str("\")\n");
    }
    source.push_str("WAIT\nRETURN\n");
    let (mut harness, mut open) = SqlHarness::start(&source);
    let mut final_messages = None;

    for index in 0..8_u8 {
        let expected = format!("db{index}");
        assert!(matches!(
            &open.payload.operation,
            SqlOperationV1::Open { logical_name, .. } if logical_name == &expected
        ));
        let messages = harness.respond(&open, open_response(&open, revision(index + 1)));
        if index < 7 {
            open = take_sql_request(messages);
        } else {
            final_messages = Some(messages);
        }
    }
    let final_messages = final_messages.expect("eighth completion resumes the ninth connect");

    assert_no_sql_request(&final_messages);
    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{final_messages:#?}"
    );
    assert!(final_messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::VmFault,
            ..
        })
    )));
    assert_eq!(harness.session.sql.connections().count(), 8);
    assert!(!harness.session.sql.has_inflight());
    for index in 0..8 {
        assert_eq!(harness.integer(index), 0);
    }
}

#[test]
fn malformed_success_response_faults_without_committing_the_open() {
    let source = "@SYSTEM_TITLE\nSQL_CONNECT \"db\"\nWAIT\nRETURN\n";
    let (mut harness, open) = SqlHarness::start(source);
    let malformed = SqlResponseV1 {
        provider: open.payload.provider,
        database: None,
        reader: None,
        result: SqlResultV1::Opened {
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            limits: era_runtime_protocol::SqlLimitsV1::FIXED,
        },
    };
    let messages = harness.respond(&open, malformed);

    assert_eq!(
        harness.session.phase(),
        RuntimePhase::Faulted,
        "{messages:#?}"
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Fault(RuntimeFault {
            code: FaultCode::ServiceFailure,
            message,
            ..
        }) if message.contains("authoritative database state")
    )));
    assert!(harness.session.sql.connection_by_key("db").is_none());
    assert!(!harness.session.sql.has_inflight());
}
