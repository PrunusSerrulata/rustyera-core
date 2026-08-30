//! End-to-end SQL MAP, snapshot, and project-boundary tests through the runtime protocol.

use super::*;

use era_protocol::{decode_canonical, encode_canonical};
use era_runtime_protocol::{
    SqlConnectionHandleV1, SqlDatabaseIdentityV1, SqlDatabaseSourceV1, SqlDatabaseStateV1,
    SqlErrorCodeV1, SqlErrorV1, SqlExecuteModeV1, SqlLimitsV1, SqlOperationKindV1, SqlOperationV1,
    SqlReaderHandleV1, SqlReaderStateV1, SqlReaderStatusV1, SqlRequestV1, SqlResponseV1,
    SqlResultV1, SqlRevisionV1,
};

struct SqlHostFixture {
    session: RuntimeSession,
    sequence: u64,
    messages: Vec<RuntimeMessage>,
}

impl SqlHostFixture {
    fn new(body: &str, resources: Vec<(&str, Vec<u8>)>) -> Self {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "sql-map-snapshot-fixture".into(),
                features: vec![RuntimeFeature::Storage],
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
                configuration_profile: None,
            }),
        );
        session
            .drive(RuntimeDriveBudget::default())
            .expect("negotiate SQL host fixture");
        assert!(
            drain(&mut session)
                .iter()
                .any(|message| { matches!(message, RuntimeMessage::ServerHello(_)) })
        );
        let profile = erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
        let mut files = vec![
            profile_configuration_file(profile),
            SubmittedFile {
                relative_path: "sql-map-snapshot.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(format!("@SYSTEM_TITLE\n{body}\nWAIT\nRETURN\n")),
                content_hash: None,
            },
        ];
        files.extend(resources.into_iter().map(|(path, bytes)| SubmittedFile {
            relative_path: path.into(),
            category: FileCategory::Resource,
            payload: FilePayload::Bytes(ProtocolBytes::new(bytes)),
            content_hash: None,
        }));
        submit(
            &mut session,
            1,
            RuntimeMessage::ProjectManifest(ProjectManifest {
                compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
                project_revision: 1,
                files,
            }),
        );
        session
            .drive(RuntimeDriveBudget::default())
            .expect("load SQL host fixture");
        let loaded = drain(&mut session);
        assert!(
            loaded.iter().any(|message| matches!(
                message,
                RuntimeMessage::ProjectLoadReport(report) if report.success
            )),
            "{loaded:#?}"
        );
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        let mut fixture = Self {
            session,
            sequence: 3,
            messages: Vec::new(),
        };
        fixture.pump();
        fixture
    }

    fn pump(&mut self) {
        for _ in 0..64 {
            self.session
                .drive(RuntimeDriveBudget::default())
                .expect("drive SQL host fixture");
            self.messages.extend(drain(&mut self.session));
            if matches!(
                self.session.phase(),
                RuntimePhase::WaitingExternal
                    | RuntimePhase::WaitingInput
                    | RuntimePhase::Faulted
                    | RuntimePhase::Stopped
                    | RuntimePhase::Ready
            ) {
                return;
            }
        }
        panic!("SQL fixture made no bounded progress: {:#?}", self.messages);
    }

    fn submit_message(&mut self, message: RuntimeMessage) {
        submit(&mut self.session, self.sequence, message);
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("SQL fixture sequence remains bounded");
        self.pump();
    }

    fn take_storage_request(&mut self) -> StorageRequest {
        let index = self
            .messages
            .iter()
            .position(|message| matches!(message, RuntimeMessage::StorageRequest(_)))
            .unwrap_or_else(|| panic!("missing SQL storage request: {:#?}", self.messages));
        let RuntimeMessage::StorageRequest(request) = self.messages.remove(index) else {
            unreachable!()
        };
        request
    }

    fn take_sql_request(&mut self) -> (ServiceRequest, SqlRequestV1) {
        let index = self
            .messages
            .iter()
            .position(|message| {
                matches!(
                    message,
                    RuntimeMessage::ServiceRequest(request)
                        if request.kind == ServiceKind::Sql
                            && request.operation == SQL_OPERATION
                )
            })
            .unwrap_or_else(|| panic!("missing SQL service request: {:#?}", self.messages));
        let RuntimeMessage::ServiceRequest(request) = self.messages.remove(index) else {
            unreachable!()
        };
        assert_eq!(request.operation_version, SQL_OPERATION_VERSION);
        assert_eq!(request.deadline_ns, None);
        let payload = decode_canonical(request.payload.as_slice()).expect("decode SQL request");
        (request, payload)
    }

    fn respond_storage(&mut self, request: &StorageRequest, result: StorageResult) {
        self.submit_message(RuntimeMessage::StorageResponse(StorageResponse {
            request_id: request.request_id,
            result,
        }));
    }

    #[allow(clippy::needless_pass_by_value)]
    fn respond_sql(&mut self, request: &ServiceRequest, response: SqlResponseV1) {
        self.submit_message(RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: request.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&response).expect("encode SQL response"),
                ),
            },
        }));
    }

    fn answer_memory_open(
        &mut self,
        durable_revision: Option<SqlRevisionV1>,
    ) -> (String, SqlConnectionHandleV1) {
        let (request, payload) = self.take_sql_request();
        let SqlOperationV1::Open {
            connection,
            logical_name,
            identity,
            revision: open_revision,
            limits,
        } = payload.operation
        else {
            panic!("expected SQL Open request")
        };
        assert_eq!(identity.source, SqlDatabaseSourceV1::Memory);
        assert_eq!(
            open_revision,
            era_runtime_protocol::SqlOpenRevisionV1::Current
        );
        assert_eq!(limits, SqlLimitsV1::FIXED);
        self.respond_sql(
            &request,
            SqlResponseV1 {
                provider: payload.provider,
                database: Some(SqlDatabaseStateV1 {
                    connection,
                    connected: true,
                    transaction_active: false,
                    durable_revision,
                }),
                reader: None,
                result: SqlResultV1::Opened {
                    sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
                    limits: SqlLimitsV1::FIXED,
                },
            },
        );
        (logical_name, connection)
    }

    fn integer(&self, index: u64) -> i64 {
        read_runtime_integer(
            self.session.vm.as_ref().expect("SQL fixture VM"),
            "RESULT",
            &[index],
            None,
        )
        .expect("read SQL fixture RESULT")
    }

    fn assert_faulted(&self) {
        assert_eq!(
            self.session.phase(),
            RuntimePhase::Faulted,
            "{:#?}",
            self.messages
        );
        assert!(
            self.messages
                .iter()
                .any(|message| matches!(message, RuntimeMessage::Fault(_))),
            "{:#?}",
            self.messages
        );
    }
}

fn revision(byte: u8) -> SqlRevisionV1 {
    SqlRevisionV1 {
        sha256: ProtocolBytes::new(vec![byte; 32]),
    }
}

fn export_snapshot_bytes(fixture: &mut SqlHostFixture, message_id: u64) -> Vec<u8> {
    fixture
        .session
        .export_state(
            message_id,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .expect("export stable SQL runtime snapshot");
    let messages = drain(&mut fixture.session);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: StateExportKind::VmSnapshot,
                result: StateExportResult::Ready { .. },
            })
        )),
        "{messages:#?}"
    );
    fixture
        .session
        .outbound_transfer
        .take()
        .expect("snapshot transfer bytes")
        .bytes
        .as_ref()
        .clone()
}

fn memory_identity() -> SqlDatabaseIdentityV1 {
    SqlDatabaseIdentityV1 {
        source: SqlDatabaseSourceV1::Memory,
        sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
        format_version: era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION,
    }
}

fn storage_read(bytes: &[u8]) -> StorageResult {
    StorageResult::Read {
        data: ProtocolBytes::new(bytes.to_vec()),
        revision: None,
    }
}

fn map_fixture(xml: Vec<u8>) -> SqlHostFixture {
    SqlHostFixture::new(
        "RESULT:0 = SQL_CONNECT(\"db\")\nRESULT:1 = SQL_IMPORT_MAP_XML(\"db\", \"translations\", \"maps/test.xml\")",
        vec![("maps/test.xml", xml)],
    )
}

fn answer_map_import(
    fixture: &mut SqlHostFixture,
    request: &ServiceRequest,
    payload: &SqlRequestV1,
    connection: SqlConnectionHandleV1,
    rows: u32,
) {
    fixture.respond_sql(
        request,
        SqlResponseV1 {
            provider: payload.provider,
            database: Some(SqlDatabaseStateV1 {
                connection,
                connected: true,
                transaction_active: false,
                durable_revision: Some(revision(2)),
            }),
            reader: None,
            result: SqlResultV1::MapImported { rows },
        },
    );
}

#[test]
fn sql_map_import_uses_resource_digest_and_preserves_rows() {
    let xml = br#"<map><p><k>a&amp;b</k><v>head<b x="1">mid</b>tail</v></p><p><k>dup</k><v>first</v></p><p><k>dup</k><v><i>second</i></v></p><p><k>missing</k></p></map>"#.to_vec();
    let mut fixture = map_fixture(xml.clone());
    let (_, connection) = fixture.answer_memory_open(Some(revision(1)));
    let storage = fixture.take_storage_request();
    assert_eq!(storage.namespace, StorageNamespace::Resource);
    assert_eq!(storage.relative_path, "maps/test.xml");
    assert!(matches!(&storage.operation, StorageOperation::Read));
    fixture.respond_storage(&storage, storage_read(&xml));

    let (service, payload) = fixture.take_sql_request();
    let SqlOperationV1::ImportMapRows {
        connection: imported_connection,
        table,
        rows,
    } = &payload.operation
    else {
        panic!("expected SQL ImportMapRows request")
    };
    assert_eq!(*imported_connection, connection);
    assert_eq!(table, "translations");
    assert_eq!(
        rows.iter()
            .map(|row| (row.key.as_str(), row.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("a&b", "head<b x=\"1\">mid</b>tail"),
            ("dup", "first"),
            ("dup", "<i>second</i>"),
        ]
    );
    answer_map_import(&mut fixture, &service, &payload, connection, 3);
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(fixture.integer(0), 0);
    assert_eq!(fixture.integer(1), 1);

    let mut mismatch = map_fixture(xml);
    mismatch.answer_memory_open(Some(revision(1)));
    let storage = mismatch.take_storage_request();
    mismatch.respond_storage(&storage, storage_read(b"<map></map>"));
    mismatch.assert_faulted();
    assert!(mismatch.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
}

#[test]
fn sql_map_import_rejects_invalid_utf8_and_oversize_resources() {
    let mut invalid_utf8 = map_fixture(vec![0xff]);
    invalid_utf8.answer_memory_open(Some(revision(1)));
    let storage = invalid_utf8.take_storage_request();
    invalid_utf8.respond_storage(&storage, storage_read(&[0xff]));
    invalid_utf8.assert_faulted();

    let maximum_bytes =
        usize::try_from(SqlLimitsV1::FIXED.maximum_map_bytes).expect("MAP byte limit fits usize");
    let oversize = vec![b' '; maximum_bytes + 1];
    let mut oversized = map_fixture(oversize.clone());
    oversized.answer_memory_open(Some(revision(1)));
    let storage = oversized.take_storage_request();
    oversized.respond_storage(&storage, storage_read(&oversize));
    oversized.assert_faulted();
    assert!(oversized.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
}

#[test]
#[allow(clippy::too_many_lines)]
fn sql_map_import_accepts_exact_size_and_row_limits_but_rejects_the_next_row() {
    let maximum_bytes =
        usize::try_from(SqlLimitsV1::FIXED.maximum_map_bytes).expect("MAP byte limit fits usize");
    let mut exact_bytes = b"<map>".to_vec();
    exact_bytes.resize(maximum_bytes - b"</map>".len(), b' ');
    exact_bytes.extend_from_slice(b"</map>");
    assert_eq!(exact_bytes.len(), maximum_bytes);
    let mut bytes_fixture = map_fixture(exact_bytes.clone());
    let (_, connection) = bytes_fixture.answer_memory_open(Some(revision(1)));
    let storage = bytes_fixture.take_storage_request();
    bytes_fixture.respond_storage(&storage, storage_read(&exact_bytes));
    let (service, payload) = bytes_fixture.take_sql_request();
    let SqlOperationV1::ImportMapRows { rows, .. } = &payload.operation else {
        panic!("expected exact-size MAP import")
    };
    assert!(rows.is_empty());
    answer_map_import(&mut bytes_fixture, &service, &payload, connection, 0);
    assert_eq!(bytes_fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(bytes_fixture.integer(1), 1);

    let row = "<p><k></k><v></v></p>";
    let maximum_rows =
        usize::try_from(SqlLimitsV1::FIXED.maximum_map_rows).expect("MAP row limit fits usize");
    let exact_rows = format!("<map>{}</map>", row.repeat(maximum_rows)).into_bytes();
    assert!(exact_rows.len() < maximum_bytes);
    let mut rows_fixture = map_fixture(exact_rows.clone());
    let (_, connection) = rows_fixture.answer_memory_open(Some(revision(1)));
    let storage = rows_fixture.take_storage_request();
    rows_fixture.respond_storage(&storage, storage_read(&exact_rows));
    let (service, payload) = rows_fixture.take_sql_request();
    let SqlOperationV1::ImportMapRows { rows, .. } = &payload.operation else {
        panic!("expected exact-row MAP import")
    };
    assert_eq!(rows.len(), maximum_rows);
    answer_map_import(
        &mut rows_fixture,
        &service,
        &payload,
        connection,
        SqlLimitsV1::FIXED.maximum_map_rows,
    );
    assert_eq!(rows_fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(rows_fixture.integer(1), 1);

    let too_many_rows = format!("<map>{}</map>", row.repeat(maximum_rows + 1)).into_bytes();
    assert!(too_many_rows.len() < maximum_bytes);
    let mut rejected = map_fixture(too_many_rows.clone());
    rejected.answer_memory_open(Some(revision(1)));
    let storage = rejected.take_storage_request();
    rejected.respond_storage(&storage, storage_read(&too_many_rows));
    rejected.assert_faulted();
    assert!(rejected.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
}

fn snapshot_reasons(
    fixture: &mut SqlHostFixture,
    message_id: u64,
) -> Vec<SnapshotIneligibleReason> {
    fixture
        .session
        .export_state(
            message_id,
            StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .expect("request SQL snapshot eligibility");
    let messages = drain(&mut fixture.session);
    let results = messages
        .into_iter()
        .filter_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                kind: StateExportKind::VmSnapshot,
                result: StateExportResult::Ineligible { reasons },
            }) => Some(reasons),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    results
        .into_iter()
        .next()
        .expect("snapshot ineligible result")
}

#[test]
#[allow(clippy::too_many_lines)]
fn real_sql_host_activity_blocks_snapshots_for_inflight_reader_transaction_and_revision() {
    let mut inflight = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
    assert_eq!(inflight.session.phase(), RuntimePhase::WaitingExternal);
    assert!(
        snapshot_reasons(&mut inflight, 500)
            .contains(&SnapshotIneligibleReason::ExternalOperationPending)
    );

    let mut reader = SqlHostFixture::new(
        "RESULT:0 = SQL_CONNECT(\"db\")\nRESULT:1 = SQL_EXECUTE_READER(\"db\", \"SELECT 1\")",
        Vec::new(),
    );
    let (_, connection) = reader.answer_memory_open(Some(revision(1)));
    let (request, payload) = reader.take_sql_request();
    let SqlOperationV1::Execute {
        connection: execute_connection,
        mode: SqlExecuteModeV1::Reader,
        ..
    } = payload.operation
    else {
        panic!("expected reader Execute request")
    };
    assert_eq!(execute_connection, connection);
    let reader_handle = SqlReaderHandleV1 {
        service_epoch: payload.provider.service_epoch,
        id: 41,
    };
    reader.respond_sql(
        &request,
        SqlResponseV1 {
            provider: payload.provider,
            database: Some(SqlDatabaseStateV1 {
                connection,
                connected: true,
                transaction_active: false,
                durable_revision: Some(revision(1)),
            }),
            reader: Some(SqlReaderStateV1 {
                reader: reader_handle,
                status: SqlReaderStatusV1::BeforeFirst,
                rows_read: 0,
            }),
            result: SqlResultV1::ReaderOpened {
                reader: reader_handle,
            },
        },
    );
    assert_eq!(reader.session.phase(), RuntimePhase::WaitingInput);
    assert!(
        snapshot_reasons(&mut reader, 501)
            .contains(&SnapshotIneligibleReason::SnapshotStateUnavailable)
    );

    let mut transaction = SqlHostFixture::new(
        "RESULT:0 = SQL_CONNECT(\"db\")\nRESULT:1 = SQL_EXECUTE_NONQUERY(\"db\", \"BEGIN\")",
        Vec::new(),
    );
    let (_, connection) = transaction.answer_memory_open(Some(revision(1)));
    let (request, payload) = transaction.take_sql_request();
    assert!(matches!(
        payload.operation,
        SqlOperationV1::Execute {
            connection: candidate,
            mode: SqlExecuteModeV1::NonQuery,
            ..
        } if candidate == connection
    ));
    transaction.respond_sql(
        &request,
        SqlResponseV1 {
            provider: payload.provider,
            database: Some(SqlDatabaseStateV1 {
                connection,
                connected: true,
                transaction_active: true,
                durable_revision: Some(revision(1)),
            }),
            reader: None,
            result: SqlResultV1::NonQuery { affected_rows: 0 },
        },
    );
    assert_eq!(transaction.session.phase(), RuntimePhase::WaitingInput);
    assert!(
        snapshot_reasons(&mut transaction, 502)
            .contains(&SnapshotIneligibleReason::SnapshotStateUnavailable)
    );

    let mut missing_revision = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
    missing_revision.answer_memory_open(None);
    assert_eq!(missing_revision.session.phase(), RuntimePhase::WaitingInput);
    assert!(
        snapshot_reasons(&mut missing_revision, 503)
            .contains(&SnapshotIneligibleReason::SnapshotStateUnavailable)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_restore_failure_keeps_the_active_sql_state_and_cleans_the_candidate() {
    let mut fixture = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"old\")", Vec::new());
    let (_, old_connection) = fixture.answer_memory_open(Some(revision(1)));
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    let old_provider = fixture.session.sql.provider();
    fixture.messages.clear();

    fixture
        .session
        .begin_sql_snapshot_restore(
            600,
            b"candidate snapshot bytes".to_vec(),
            vec![
                crate::runtime_snapshot::SqlConnectionSnapshot {
                    logical_name: "alpha".into(),
                    identity: memory_identity(),
                    durable_revision: revision(7),
                },
                crate::runtime_snapshot::SqlConnectionSnapshot {
                    logical_name: "beta".into(),
                    identity: memory_identity(),
                    durable_revision: revision(8),
                },
            ],
        )
        .expect("begin exact SQL snapshot candidate");
    fixture.messages.extend(drain(&mut fixture.session));

    let (alpha_request, alpha_payload) = fixture.take_sql_request();
    let SqlOperationV1::Open {
        connection: alpha_connection,
        logical_name,
        revision: era_runtime_protocol::SqlOpenRevisionV1::Exact(alpha_revision),
        ..
    } = alpha_payload.operation
    else {
        panic!("expected first exact SQL Open")
    };
    assert_eq!(logical_name, "alpha");
    assert_eq!(alpha_revision, revision(7));
    assert_ne!(
        alpha_payload.provider.service_epoch,
        old_provider.service_epoch
    );
    fixture.respond_sql(
        &alpha_request,
        SqlResponseV1 {
            provider: alpha_payload.provider,
            database: Some(SqlDatabaseStateV1 {
                connection: alpha_connection,
                connected: true,
                transaction_active: false,
                durable_revision: Some(revision(7)),
            }),
            reader: None,
            result: SqlResultV1::Opened {
                sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
                limits: SqlLimitsV1::FIXED,
            },
        },
    );
    assert_eq!(
        fixture
            .session
            .sql
            .connection_by_key("old")
            .expect("active old connection remains during candidate restore")
            .handle,
        old_connection
    );

    let (beta_request, beta_payload) = fixture.take_sql_request();
    let SqlOperationV1::Open {
        logical_name,
        revision: era_runtime_protocol::SqlOpenRevisionV1::Exact(beta_revision),
        ..
    } = beta_payload.operation
    else {
        panic!("expected second exact SQL Open")
    };
    assert_eq!(logical_name, "beta");
    assert_eq!(beta_revision, revision(8));
    fixture.respond_sql(
        &beta_request,
        SqlResponseV1 {
            provider: beta_payload.provider,
            database: None,
            reader: None,
            result: SqlResultV1::Error {
                error: SqlErrorV1 {
                    code: SqlErrorCodeV1::RevisionMissing,
                    operation: SqlOperationKindV1::Open,
                    context: Vec::new(),
                    sqlite_code: None,
                    sqlite_message: None,
                },
            },
        },
    );

    let active = fixture
        .session
        .sql
        .connection_by_key("old")
        .expect("failed restore preserves active SQL state");
    assert_eq!(active.handle, old_connection);
    assert_eq!(active.durable_revision.as_ref(), Some(&revision(1)));
    assert!(fixture.session.sql.connection_by_key("alpha").is_none());
    assert!(fixture.session.ready_sql_snapshot_restore.is_none());
    assert!(fixture.messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(rejection)
            if rejection.code == CommandErrorCode::InvalidValue
    )));
    let (_, cleanup_payload) = fixture.take_sql_request();
    assert_eq!(cleanup_payload.provider, alpha_payload.provider);
    assert!(matches!(
        cleanup_payload.operation,
        SqlOperationV1::Disconnect { connection } if connection == alpha_connection
    ));
}

#[test]
fn exact_restore_swaps_only_after_the_provider_reopens_the_recorded_revision() {
    let mut fixture = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
    let (_, old_connection) = fixture.answer_memory_open(Some(revision(4)));
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    let old_provider = fixture.session.sql.provider();
    fixture.messages.clear();
    let bytes = export_snapshot_bytes(&mut fixture, 610);

    fixture
        .session
        .start_vm_snapshot(611, &bytes)
        .expect("begin exact restore from a real runtime snapshot");
    fixture.messages.extend(drain(&mut fixture.session));
    assert_eq!(
        fixture
            .session
            .sql
            .connection_by_key("db")
            .expect("old SQL remains until exact Open succeeds")
            .handle,
        old_connection
    );
    let (request, payload) = fixture.take_sql_request();
    let SqlOperationV1::Open {
        connection: candidate_connection,
        logical_name,
        revision: era_runtime_protocol::SqlOpenRevisionV1::Exact(exact_revision),
        ..
    } = payload.operation
    else {
        panic!("expected exact SQL Open")
    };
    assert_eq!(logical_name, "db");
    assert_eq!(exact_revision, revision(4));
    assert_ne!(payload.provider, old_provider);
    fixture.respond_sql(
        &request,
        SqlResponseV1 {
            provider: payload.provider,
            database: Some(SqlDatabaseStateV1 {
                connection: candidate_connection,
                connected: true,
                transaction_active: false,
                durable_revision: Some(revision(4)),
            }),
            reader: None,
            result: SqlResultV1::Opened {
                sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
                limits: SqlLimitsV1::FIXED,
            },
        },
    );

    let active = fixture
        .session
        .sql
        .connection_by_key("db")
        .expect("candidate SQL was committed");
    assert_eq!(active.handle, candidate_connection);
    assert_eq!(active.durable_revision.as_ref(), Some(&revision(4)));
    assert_ne!(active.handle, old_connection);
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    let (_, cleanup) = fixture.take_sql_request();
    assert_eq!(cleanup.provider, old_provider);
    assert!(matches!(
        cleanup.operation,
        SqlOperationV1::Disconnect { connection } if connection == old_connection
    ));
}

#[test]
fn empty_sql_snapshot_preserves_old_state_on_vm_failure_then_cleans_it_on_commit() {
    let mut fixture = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"old\")", Vec::new());
    let (_, old_connection) = fixture.answer_memory_open(Some(revision(5)));
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    let old_provider = fixture.session.sql.provider();
    fixture.messages.clear();
    let bytes = export_snapshot_bytes(&mut fixture, 620);
    let mut empty = crate::runtime_snapshot::decode(&bytes, usize::MAX)
        .expect("decode exported runtime snapshot");
    empty.sql.connections.clear();

    let mut invalid = empty;
    invalid.vm_snapshot[0] ^= 0xff;
    let invalid = crate::runtime_snapshot::encode(&invalid)
        .expect("encode invalid embedded VM with an empty SQL snapshot");
    fixture
        .session
        .start_vm_snapshot(621, &invalid)
        .expect("reject invalid embedded VM without replacing SQL");
    let rejected = drain(&mut fixture.session);
    assert!(
        rejected.iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(rejection)
                if rejection.code == CommandErrorCode::InvalidValue
        )),
        "{rejected:#?}"
    );
    assert_eq!(
        fixture
            .session
            .sql
            .connection_by_key("old")
            .expect("invalid empty snapshot preserves old SQL")
            .handle,
        old_connection
    );

    let mut empty = crate::runtime_snapshot::decode(&bytes, usize::MAX)
        .expect("decode exported runtime snapshot again");
    empty.sql.connections.clear();
    let empty = crate::runtime_snapshot::encode(&empty)
        .expect("encode valid runtime snapshot with empty SQL state");
    fixture
        .session
        .start_vm_snapshot(622, &empty)
        .expect("commit empty SQL snapshot after VM validation");
    fixture.messages.extend(drain(&mut fixture.session));
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(fixture.session.sql.connections().count(), 0);
    let (_, cleanup) = fixture.take_sql_request();
    assert_eq!(cleanup.provider, old_provider);
    assert!(matches!(
        cleanup.operation,
        SqlOperationV1::Disconnect { connection } if connection == old_connection
    ));
}

#[test]
fn hot_reload_rejects_a_changed_resource_seed_digest_without_replacing_the_project() {
    let seed = b"sqlite-seed-v1".to_vec();
    let mut fixture = SqlHostFixture::new(
        "RESULT:0 = SQL_CONNECT(\"seeded\", \"Data Source=db/seed.db\")",
        vec![("db/seed.db", seed.clone())],
    );
    let storage = fixture.take_storage_request();
    assert_eq!(storage.namespace, StorageNamespace::Resource);
    assert_eq!(storage.relative_path, "db/seed.db");
    fixture.respond_storage(&storage, storage_read(&seed));
    let (request, payload) = fixture.take_sql_request();
    let SqlOperationV1::Open {
        connection,
        identity,
        ..
    } = &payload.operation
    else {
        panic!("expected seeded SQL Open")
    };
    let SqlDatabaseSourceV1::ResourceSeed(resource_seed) = &identity.source else {
        panic!("seeded connection must use ResourceSeed identity")
    };
    assert_eq!(resource_seed.resource_id, "db/seed.db");
    assert_eq!(resource_seed.sha256.as_slice().len(), 32);
    let connection = *connection;
    fixture.respond_sql(
        &request,
        SqlResponseV1 {
            provider: payload.provider,
            database: Some(SqlDatabaseStateV1 {
                connection,
                connected: true,
                transaction_active: false,
                durable_revision: Some(revision(3)),
            }),
            reader: None,
            result: SqlResultV1::Opened {
                sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
                limits: SqlLimitsV1::FIXED,
            },
        },
    );
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    fixture.messages.clear();

    fixture
        .session
        .reload_project(
            700,
            &ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: vec![FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "db/seed.db".into(),
                        category: FileCategory::Resource,
                        payload: FilePayload::Bytes(ProtocolBytes::new(b"sqlite-seed-v2".to_vec())),
                        content_hash: None,
                    },
                }],
            },
        )
        .expect("evaluate Resource-seed hot reload");
    let messages = drain(&mut fixture.session);
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::CommandRejected(rejection)
                if rejection.code == CommandErrorCode::InvalidState
                    && rejection.message.contains("Resource seed")
        )),
        "{messages:#?}"
    );
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(
        fixture
            .session
            .project_snapshot
            .as_ref()
            .expect("old project retained")
            .manifest
            .project_revision,
        1
    );
    assert_eq!(
        fixture
            .session
            .sql
            .connection_by_key("seeded")
            .expect("seeded connection retained")
            .handle,
        connection
    );
}

fn next_snake_project(revision: u64) -> ProjectManifest {
    let profile = erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake;
    ProjectManifest {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(profile),
        project_revision: revision,
        files: vec![
            profile_configuration_file(profile),
            SubmittedFile {
                relative_path: "next.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
        ],
    }
}

fn open_two_connections(tail: &str) -> (SqlHostFixture, Vec<(String, SqlConnectionHandleV1)>) {
    let mut fixture = SqlHostFixture::new(
        &format!("RESULT:0 = SQL_CONNECT(\"beta\")\nRESULT:1 = SQL_CONNECT(\"alpha\")\n{tail}"),
        Vec::new(),
    );
    let first = fixture.answer_memory_open(Some(revision(1)));
    let second = fixture.answer_memory_open(Some(revision(2)));
    assert_eq!(first.0, "beta");
    assert_eq!(second.0, "alpha");
    (fixture, vec![first, second])
}

fn cleanup_connections(messages: &[RuntimeMessage]) -> Vec<(usize, SqlRequestV1)> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.kind == ServiceKind::Sql && request.operation == SQL_OPERATION =>
            {
                let payload: SqlRequestV1 =
                    decode_canonical(request.payload.as_slice()).expect("decode cleanup SQL");
                matches!(&payload.operation, SqlOperationV1::Disconnect { .. })
                    .then_some((index, payload))
            }
            _ => None,
        })
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn cold_switch_and_shutdown_emit_sorted_disconnects_before_their_terminal_messages() {
    let (mut cold, opened) = open_two_connections("THROW switch-ready");
    assert_eq!(
        cold.session.phase(),
        RuntimePhase::Faulted,
        "{:#?}",
        cold.messages
    );
    let old_provider = cold.session.sql.provider();
    let beta = opened[0].1;
    let alpha = opened[1].1;
    cold.messages.clear();
    cold.submit_message(RuntimeMessage::ProjectManifest(next_snake_project(2)));
    assert_eq!(
        cold.session.phase(),
        RuntimePhase::Ready,
        "{:#?}",
        cold.messages
    );
    let cleanup = cleanup_connections(&cold.messages);
    assert_eq!(cleanup.len(), 2, "{:#?}", cold.messages);
    assert_eq!(cleanup[0].1.provider, old_provider);
    assert_eq!(cleanup[1].1.provider, old_provider);
    assert!(matches!(
        &cleanup[0].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == alpha
    ));
    assert!(matches!(
        &cleanup[1].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == beta
    ));
    let report = cold
        .messages
        .iter()
        .position(|message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success))
        .expect("cold project load report");
    assert!(cleanup.iter().all(|(index, _)| *index < report));
    assert!(cold.session.sql.connections().next().is_none());
    assert_ne!(cold.session.sql.provider(), old_provider);

    let (mut shutdown, opened) = open_two_connections("");
    assert_eq!(shutdown.session.phase(), RuntimePhase::WaitingInput);
    let old_provider = shutdown.session.sql.provider();
    let beta = opened[0].1;
    let alpha = opened[1].1;
    shutdown.messages.clear();
    shutdown.submit_message(RuntimeMessage::ShutdownRequest(ShutdownRequest {
        graceful: true,
    }));
    assert_eq!(shutdown.session.phase(), RuntimePhase::Stopped);
    let cleanup = cleanup_connections(&shutdown.messages);
    assert_eq!(cleanup.len(), 2, "{:#?}", shutdown.messages);
    assert_eq!(cleanup[0].1.provider, old_provider);
    assert_eq!(cleanup[1].1.provider, old_provider);
    assert!(matches!(
        &cleanup[0].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == alpha
    ));
    assert!(matches!(
        &cleanup[1].1.operation,
        SqlOperationV1::Disconnect { connection } if *connection == beta
    ));
    let ready = shutdown
        .messages
        .iter()
        .position(|message| matches!(message, RuntimeMessage::ShutdownReady(_)))
        .expect("shutdown ready message");
    assert!(cleanup.iter().all(|(index, _)| *index < ready));
    let RuntimeMessage::ShutdownReady(ready_message) = &shutdown.messages[ready] else {
        unreachable!()
    };
    assert_eq!(ready_message.pending_operations_cancelled, 1);
    assert!(shutdown.session.sql.connections().next().is_none());
    assert_ne!(shutdown.session.sql.provider(), old_provider);
}
