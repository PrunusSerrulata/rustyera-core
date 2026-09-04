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
