use super::*;

use era_protocol::{decode_canonical, encode_canonical};
use era_runtime_protocol::{
    SqlConnectionHandleV1, SqlDatabaseIdentityV1, SqlDatabaseSourceV1, SqlDatabaseStateV1,
    SqlErrorCodeV1, SqlErrorV1, SqlExecuteModeV1, SqlLimitsV1, SqlOperationKindV1, SqlOperationV1,
    SqlReaderHandleV1, SqlReaderStateV1, SqlReaderStatusV1, SqlRequestV1, SqlResponseV1,
    SqlResultV1, SqlRevisionV1,
};
use sha2::{Digest as _, Sha256};

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
                payload: FilePayload::Utf8(format!(
                    "@SYSTEM_TITLE\n{body}\nWAIT\nRETURN\n@SHOW_SHOP\nWAIT\nRETURN\n"
                )),
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
    let bytes = fixture
        .session
        .outbound_transfer
        .take()
        .expect("snapshot transfer bytes")
        .bytes;
    bytes.copy_range(0..bytes.len())
}

fn assert_standard_save(bytes: &[u8]) {
    assert!(matches!(
        era_runtime_save::inspect_metadata(
            bytes,
            true,
            era_runtime_save::SaveCodecLimits::default()
        ),
        Ok(era_runtime_save::SaveMetadataInspection::Complete { .. })
    ));
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
