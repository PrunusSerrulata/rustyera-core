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

#[test]
fn resource_seed_owned_identity_isolated_across_a_b_a_projects_by_actual_sha256() {
    let seed_a = b"resource-seed-a".to_vec();
    let seed_b = b"resource-seed-b".to_vec();
    let snapshot = crate::runtime_snapshot::SqlConnectionSnapshot {
        logical_name: "seeded".into(),
        identity: SqlDatabaseIdentityV1 {
            source: SqlDatabaseSourceV1::ResourceSeed(era_runtime_protocol::SqlResourceSeedV1 {
                resource_id: "db/seed.db".into(),
                sha256: ProtocolBytes::new(Sha256::digest(&seed_a).to_vec()),
            }),
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            format_version: era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION,
        },
        durable_revision: revision(9),
    };

    let project_a = SqlHostFixture::new("", vec![("db/seed.db", seed_a.clone())]);
    assert!(
        project_a
            .session
            .validate_exact_sql_restore(std::slice::from_ref(&snapshot))
            .is_ok()
    );
    let project_b = SqlHostFixture::new("", vec![("db/seed.db", seed_b)]);
    assert!(
        project_b
            .session
            .validate_exact_sql_restore(std::slice::from_ref(&snapshot))
            .is_err()
    );
    let project_a_again = SqlHostFixture::new("", vec![("db/seed.db", seed_a)]);
    assert!(
        project_a_again
            .session
            .validate_exact_sql_restore(std::slice::from_ref(&snapshot))
            .is_ok()
    );
}

#[test]
fn host_owned_resource_seed_defers_exact_sha_verification_to_the_provider() {
    let seed = b"external-resource-seed".to_vec();
    let snapshot = crate::runtime_snapshot::SqlConnectionSnapshot {
        logical_name: "seeded".into(),
        identity: SqlDatabaseIdentityV1 {
            source: SqlDatabaseSourceV1::ResourceSeed(era_runtime_protocol::SqlResourceSeedV1 {
                resource_id: "db/seed.db".into(),
                sha256: ProtocolBytes::new(Sha256::digest(&seed).to_vec()),
            }),
            sqlite_version: era_runtime_protocol::SQL_SQLITE_VERSION.into(),
            format_version: era_runtime_protocol::SQL_DATABASE_FORMAT_VERSION,
        },
        durable_revision: revision(9),
    };
    let mut fixture = SqlHostFixture::new("", vec![("db/seed.db", seed.clone())]);
    {
        let project = fixture.session.project_snapshot.as_mut().unwrap();
        let manifest = std::sync::Arc::make_mut(&mut project.manifest);
        let file = manifest
            .files
            .iter_mut()
            .find(|file| file.relative_path == "db/seed.db")
            .unwrap();
        file.payload = FilePayload::ExternalResource(era_runtime_protocol::ExternalResource {
            byte_length: seed.len() as u64,
            image_metadata: None,
        });
        file.content_hash = Some(ProtocolBytes::new(blake3::hash(&seed).as_bytes().to_vec()));
    }

    assert!(
        fixture
            .session
            .validate_exact_sql_restore(std::slice::from_ref(&snapshot))
            .is_ok()
    );
    {
        let project = fixture.session.project_snapshot.as_mut().unwrap();
        let file = std::sync::Arc::make_mut(&mut project.manifest)
            .files
            .iter_mut()
            .find(|file| file.relative_path == "db/seed.db")
            .unwrap();
        file.payload = FilePayload::Bytes(ProtocolBytes::new(Vec::new()));
    }
    assert!(
        fixture
            .session
            .validate_exact_sql_restore(std::slice::from_ref(&snapshot))
            .is_ok()
    );
    {
        let project = fixture.session.project_snapshot.as_mut().unwrap();
        std::sync::Arc::make_mut(&mut project.manifest)
            .files
            .iter_mut()
            .find(|file| file.relative_path == "db/seed.db")
            .unwrap()
            .content_hash = None;
    }
    assert!(
        fixture
            .session
            .validate_exact_sql_restore(std::slice::from_ref(&snapshot))
            .is_err()
    );
}
