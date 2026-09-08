#[test]
#[allow(clippy::too_many_lines)]
fn sql_state_blocks_vm_snapshots_but_not_stable_traditional_save_exports() {
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
    assert_standard_save(&export_traditional_save_bytes(&mut reader, 505));

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
    assert_standard_save(&export_traditional_save_bytes(&mut transaction, 506));

    let mut missing_revision = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
    missing_revision.answer_memory_open(None);
    assert_eq!(missing_revision.session.phase(), RuntimePhase::WaitingInput);
    assert!(
        snapshot_reasons(&mut missing_revision, 503)
            .contains(&SnapshotIneligibleReason::SnapshotStateUnavailable)
    );
    assert_standard_save(&export_traditional_save_bytes(&mut missing_revision, 507));
}

#[test]
fn snake_savedata_writes_bare_1808_during_an_active_sql_transaction() {
    let mut fixture = SqlHostFixture::new(
        "RESULT:0 = SQL_CONNECT(\"db\")\nRESULT:1 = SQL_EXECUTE_NONQUERY(\"db\", \"BEGIN\")\nSAVEDATA 1, \"blocked\"",
        Vec::new(),
    );
    let (_, connection) = fixture.answer_memory_open(Some(revision(1)));
    let (request, payload) = fixture.take_sql_request();
    fixture.respond_sql(
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
    assert_eq!(fixture.session.phase(), RuntimePhase::WaitingExternal);
    let write = fixture.take_storage_request();
    assert_eq!(write.namespace, StorageNamespace::Save);
    let StorageOperation::Write { data, .. } = write.operation else {
        panic!("SAVEDATA must issue a save write")
    };
    assert_standard_save(data.as_slice());
}

#[test]
fn bare_ordinary_load_preserves_live_global_rng_and_sql_state() {
    let mut fixture = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
    let (_, connection) = fixture.answer_memory_open(Some(revision(9)));
    let bytes = export_traditional_save_bytes(&mut fixture, 508);
    write_runtime_integer(
        fixture.session.vm.as_mut().unwrap(),
        "GLOBAL",
        &[0],
        None,
        77,
    )
    .unwrap();
    let mut live_rng = fixture
        .session
        .vm
        .as_ref()
        .unwrap()
        .export_random_state()
        .unwrap();
    live_rng[0] ^= 0x1234;
    fixture
        .session
        .vm
        .as_mut()
        .unwrap()
        .restore_random_state(&live_rng)
        .unwrap();
    fixture.messages.clear();

    fixture
        .session
        .complete_ordinary_load(509, &bytes, None)
        .expect("load bare interoperable ordinary save into the live VM");
    fixture.messages.extend(drain(&mut fixture.session));

    assert_eq!(
        read_runtime_integer(fixture.session.vm.as_ref().unwrap(), "GLOBAL", &[0], None).unwrap(),
        77
    );
    assert_eq!(
        fixture
            .session
            .vm
            .as_ref()
            .unwrap()
            .export_random_state()
            .unwrap(),
        live_rng
    );
    assert_eq!(
        fixture
            .session
            .sql
            .connection_by_key("db")
            .expect("bare load preserves the live SQL connection")
            .handle,
        connection
    );
    assert!(fixture.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql
    )));
    assert!(fixture.messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
            if code == "runtime.interoperable_save_external_state_preserved"
    )));
}

#[test]
fn private_envelope_magic_is_rejected_without_mutating_runtime_or_slots() {
    let mut fixture = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
    let (_, connection) = fixture.answer_memory_open(Some(revision(10)));
    let ordinary = fixture.integer(0);
    write_runtime_integer(
        fixture.session.vm.as_mut().unwrap(),
        "GLOBAL",
        &[0],
        None,
        88,
    )
    .unwrap();
    let rng = fixture
        .session
        .vm
        .as_ref()
        .unwrap()
        .export_random_state()
        .unwrap();
    fixture.session.load_slot_paths = vec!["save10.sav".into()];
    fixture
        .session
        .occupied_slot_paths
        .insert("save10.sav".into());
    fixture
        .session
        .slot_change_tokens
        .insert("save10.sav".into(), "revision-10".into());
    fixture
        .session
        .slot_labels
        .insert("save10.sav".into(), "slot ten".into());
    let slot_state = (
        fixture.session.load_slot_paths.clone(),
        fixture.session.occupied_slot_paths.clone(),
        fixture.session.slot_change_tokens.clone(),
        fixture.session.slot_labels.clone(),
    );
    fixture.messages.clear();

    fixture
        .session
        .complete_ordinary_load(510, b"RERASAV\0", None)
        .expect("snake load failure is reported without replacing live state");
    fixture.messages.extend(drain(&mut fixture.session));

    assert_eq!(fixture.integer(0), ordinary);
    assert_eq!(
        read_runtime_integer(fixture.session.vm.as_ref().unwrap(), "GLOBAL", &[0], None).unwrap(),
        88
    );
    assert_eq!(
        fixture
            .session
            .vm
            .as_ref()
            .unwrap()
            .export_random_state()
            .unwrap(),
        rng
    );
    assert_eq!(
        fixture
            .session
            .sql
            .connection_by_key("db")
            .expect("failed load preserves SQL")
            .handle,
        connection
    );
    assert_eq!(
        (
            fixture.session.load_slot_paths.clone(),
            fixture.session.occupied_slot_paths.clone(),
            fixture.session.slot_change_tokens.clone(),
            fixture.session.slot_labels.clone(),
        ),
        slot_state
    );
    assert!(fixture.messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
            if code == "runtime.snake_save_restore_failed"
    )));
    assert!(fixture.messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
            if code == "runtime.interoperable_save_external_state_preserved"
    )));
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_restore_failure_keeps_the_active_sql_state_and_cleans_the_candidate() {
    for legacy_resource in [false, true] {
        let seed = b"legacy-resource-seed".to_vec();
        let mut fixture = SqlHostFixture::new(
            "RESULT:0 = SQL_CONNECT(\"old\")",
            vec![("db/seed.db", seed.clone())],
        );
        let mut candidate_identity = memory_identity();
        if legacy_resource {
            candidate_identity.sqlite_version = "3.53.0".into();
            candidate_identity.source =
                SqlDatabaseSourceV1::ResourceSeed(era_runtime_protocol::SqlResourceSeedV1 {
                    resource_id: "db/seed.db".into(),
                    sha256: ProtocolBytes::new(Sha256::digest(&seed).to_vec()),
                });
        }
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
                        identity: candidate_identity.clone(),
                        durable_revision: revision(7),
                    },
                    crate::runtime_snapshot::SqlConnectionSnapshot {
                        logical_name: "beta".into(),
                        identity: candidate_identity.clone(),
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
            identity,
            revision: era_runtime_protocol::SqlOpenRevisionV1::Exact(alpha_revision),
            ..
        } = alpha_payload.operation
        else {
            panic!("expected first exact SQL Open")
        };
        assert_eq!(logical_name, "alpha");
        assert_eq!(
            identity.sqlite_version,
            era_runtime_protocol::SQL_SQLITE_VERSION
        );
        assert_eq!(identity.source, candidate_identity.source);
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
        assert!(fixture.session.pending_sql_snapshot_restore.is_none());
    }
}

fn legacy_sql_snapshot_fixture(resource: bool) -> (SqlHostFixture, SqlConnectionHandleV1) {
    let seed = b"legacy-resource-seed".to_vec();
    let mut fixture = SqlHostFixture::new(
        if resource {
            "RESULT:0 = SQL_CONNECT(\"db\", \"Data Source=db/seed.db\")"
        } else {
            "RESULT:0 = SQL_CONNECT(\"db\")"
        },
        if resource {
            vec![("db/seed.db", seed.clone())]
        } else {
            Vec::new()
        },
    );
    let old_connection = if resource {
        let storage = fixture.take_storage_request();
        fixture.respond_storage(&storage, storage_read(&seed));
        let (request, payload) = fixture.take_sql_request();
        let SqlOperationV1::Open { connection, .. } = payload.operation else {
            panic!("expected resource Open");
        };
        fixture.respond_sql(
            &request,
            SqlResponseV1 {
                provider: payload.provider,
                database: Some(SqlDatabaseStateV1 {
                    connection,
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
        connection
    } else {
        fixture.answer_memory_open(Some(revision(4))).1
    };
    (fixture, old_connection)
}

fn assert_no_pending_sql_candidate(fixture: &SqlHostFixture) {
    assert!(fixture.session.pending_sql_snapshot_restore.is_none());
    assert!(fixture.session.ready_sql_snapshot_restore.is_none());
    assert!(!fixture.messages.iter().any(|message| matches!(message, RuntimeMessage::ServiceRequest(request) if request.kind == ServiceKind::Sql)));
}

#[test]
fn legacy_sql_snapshot_reopens_only_with_the_actual_current_engine() {
    for resource in [false, true] {
        for reported_version in ["3.53.4", "3.53.0", "9.0.0"] {
            let (mut fixture, old_connection) = legacy_sql_snapshot_fixture(resource);
            fixture.messages.clear();
            let original = export_snapshot_bytes(&mut fixture, 710);
            let mut snapshot = crate::runtime_snapshot::decode(&original, usize::MAX).unwrap();
            snapshot.sql.connections[0].identity.sqlite_version = "3.53.0".into();
            snapshot.sql.connections[0].identity.format_version = 1;
            let bytes = crate::runtime_snapshot::encode(&snapshot).unwrap();
            fixture.session.start_vm_snapshot(711, &bytes).unwrap();
            fixture.messages.extend(drain(&mut fixture.session));
            assert_eq!(
                fixture
                    .session
                    .pending_sql_snapshot_restore
                    .as_ref()
                    .unwrap()
                    .bytes,
                bytes
            );
            assert_eq!(
                fixture.session.sql.connection_by_key("db").unwrap().handle,
                old_connection
            );
            let (request, payload) = fixture.take_sql_request();
            let SqlOperationV1::Open {
                connection,
                identity,
                revision: era_runtime_protocol::SqlOpenRevisionV1::Exact(exact),
                ..
            } = payload.operation
            else {
                panic!("expected exact Open");
            };
            assert_eq!(identity.sqlite_version, "3.53.4");
            assert_eq!(identity.format_version, 1);
            assert_eq!(identity.source, snapshot.sql.connections[0].identity.source);
            assert_eq!(exact, revision(4));
            fixture.respond_sql(
                &request,
                SqlResponseV1 {
                    provider: payload.provider,
                    database: Some(SqlDatabaseStateV1 {
                        connection,
                        connected: true,
                        transaction_active: false,
                        durable_revision: Some(revision(4)),
                    }),
                    reader: None,
                    result: SqlResultV1::Opened {
                        sqlite_version: reported_version.into(),
                        limits: SqlLimitsV1::FIXED,
                    },
                },
            );
            let active = fixture.session.sql.connection_by_key("db").unwrap();
            if reported_version == "3.53.4" {
                assert_eq!(active.handle, connection);
                assert_eq!(active.identity.sqlite_version, "3.53.4");
                assert_eq!(active.durable_revision.as_ref(), Some(&revision(4)));
                assert_eq!(fixture.session.phase(), RuntimePhase::WaitingInput);
            } else {
                assert_eq!(active.handle, old_connection);
                assert!(fixture.messages.iter().any(|message| matches!(
                    message, RuntimeMessage::CommandRejected(rejection)
                        if rejection.code == CommandErrorCode::VersionMismatch
                )));
                let (_, cleanup) = fixture.take_sql_request();
                assert_eq!(cleanup.provider, payload.provider);
                assert!(
                    matches!(cleanup.operation, SqlOperationV1::Disconnect { connection: cleaned } if cleaned == connection && cleaned != old_connection)
                );
                assert_no_pending_sql_candidate(&fixture);
            }
            // The caller-owned original container remains a legacy snapshot.
            let unchanged = crate::runtime_snapshot::decode(&bytes, usize::MAX).unwrap();
            assert_eq!(
                unchanged.sql.connections[0].identity.sqlite_version,
                "3.53.0"
            );
            assert_eq!(unchanged.sql.connections[0].durable_revision, revision(4));
        }
    }
}

#[test]
fn sql_snapshot_migration_rejects_unknown_versions_and_formats_before_open() {
    for (version, format) in [("3.53.1", 1), ("9.0.0", 1), ("3.53.0", 2), ("3.53.4", 2)] {
        let mut fixture = SqlHostFixture::new("RESULT:0 = SQL_CONNECT(\"db\")", Vec::new());
        let (_, old_connection) = fixture.answer_memory_open(Some(revision(4)));
        let original = export_snapshot_bytes(&mut fixture, 720);
        let mut snapshot = crate::runtime_snapshot::decode(&original, usize::MAX).unwrap();
        snapshot.sql.connections[0].identity.sqlite_version = version.into();
        snapshot.sql.connections[0].identity.format_version = format;
        let bytes = crate::runtime_snapshot::encode(&snapshot).unwrap();
        fixture.messages.clear();
        fixture.session.start_vm_snapshot(721, &bytes).unwrap();
        fixture.messages.extend(drain(&mut fixture.session));
        assert_no_pending_sql_candidate(&fixture);
        assert_eq!(
            fixture.session.sql.connection_by_key("db").unwrap().handle,
            old_connection
        );
        assert!(fixture.messages.iter().any(|message| matches!(
            message, RuntimeMessage::CommandRejected(rejection)
                if rejection.code == CommandErrorCode::InvalidValue
        )));
    }
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
