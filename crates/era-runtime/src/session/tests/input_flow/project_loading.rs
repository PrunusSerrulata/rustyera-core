#[test]
#[allow(clippy::too_many_lines)]
fn project_resource_metadata_is_frontend_decoded_before_load_commit() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "resource-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/sprites.csv".into(),
                    category: FileCategory::ResourceManifest,
                    payload: FilePayload::Utf8("FACE,face.png".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
    let request_id = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("image metadata request");
    submit(
        &mut session,
        2,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 32,
                        height: 16,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((32, 16))
    );

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 5, 6])),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let reload_messages = drain(&mut session);
    let reload_request = reload_messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("changed image metadata request");
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((32, 16)),
        "the live graph must not change before candidate metadata commits"
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: reload_request,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 64,
                        height: 24,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success && report.project_revision == 2)
        }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .and_then(|project| project.resource_graph.sprite("face"))
            .map(|sprite| (sprite.width, sprite.height)),
        Some((64, 24))
    );
    let committed_identity = session.project_snapshot.as_ref().unwrap().project_identity;
    let committed_payloads = session
        .project_snapshot
        .as_ref()
        .unwrap()
        .manifest
        .files
        .iter()
        .map(|file| file.payload.clone())
        .collect::<Vec<_>>();
    let committed_artifact =
        std::ptr::from_ref(session.artifact.as_ref().unwrap().artifact()).addr();

    submit(
        &mut session,
        5,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 2,
            target_revision: 3,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "resources/face.png".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("second changed image metadata request");
    submit(
        &mut session,
        6,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: failed_request,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "decoder.invalid".into(),
                    message: "invalid image".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed = drain(&mut session);
    assert!(failed.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if !report.success && report.project_revision == 3)
        }));
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .map(|project| project.manifest.project_revision),
        Some(2),
        "failed candidate metadata must leave the previous project authoritative"
    );
    assert_eq!(
        session.project_snapshot.as_ref().unwrap().project_identity,
        committed_identity
    );
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .manifest
            .files
            .iter()
            .map(|file| file.payload.clone())
            .collect::<Vec<_>>(),
        committed_payloads
    );
    assert_eq!(
        std::ptr::from_ref(session.artifact.as_ref().unwrap().artifact()).addr(),
        committed_artifact
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn cold_project_metadata_is_transactional_and_low_memory_commit_is_sparse() {
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    let mut client = capabilities();
    client.graphics = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "transaction-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "old.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL OLD\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(
        |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    ));
    let old_snapshot = session.project_snapshot.as_ref().unwrap();
    let old_identity = old_snapshot.project_identity;
    let old_revision = old_snapshot.manifest.project_revision;
    let old_payload = old_snapshot.manifest.files[0].payload.clone();
    let old_artifact = std::ptr::from_ref(session.artifact.as_ref().unwrap().artifact()).addr();
    session.compiled_project_cache = Some(Arc::new(vec![1, 2, 3]));
    session.full_project_file = Some(Arc::new(crate::compiled_cache::ContainerBytes::new(
        false,
        vec![4, 5, 6],
    )));
    session.client_preferences = Some(ClientPreferenceLayers {
        project_revision: 1,
        global: Vec::new(),
        project: Vec::new(),
    });

    let next_manifest = |project_revision| ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision,
        files: vec![
            SubmittedFile {
                relative_path: "next.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL NEXT\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/sprites.csv".into(),
                category: FileCategory::ResourceManifest,
                payload: FilePayload::Utf8("FACE,face.png".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "resources/face.png".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
                content_hash: None,
            },
        ],
    };
    submit(
        &mut session,
        2,
        RuntimeMessage::ProjectManifest(next_manifest(2)),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let failed_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("candidate image metadata request");
    assert_eq!(
        session.project_snapshot.as_ref().unwrap().project_identity,
        old_identity
    );
    assert_eq!(
        session.compiled_project_cache.as_deref(),
        Some(&vec![1, 2, 3])
    );
    assert_eq!(
        session
            .full_project_file
            .as_ref()
            .map(|bytes| bytes.copy_range(0..bytes.len())),
        Some(vec![4, 5, 6])
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: failed_request,
            result: ServiceResult::Error {
                error: era_runtime_protocol::ServiceError {
                    code: "decoder.invalid".into(),
                    message: "invalid image".into(),
                },
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(
        |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if !report.success)
    ));
    let retained = session.project_snapshot.as_ref().unwrap();
    assert_eq!(session.phase, RuntimePhase::Ready);
    assert_eq!(retained.manifest.project_revision, old_revision);
    assert_eq!(retained.project_identity, old_identity);
    assert_eq!(retained.manifest.files[0].payload, old_payload);
    assert_eq!(
        std::ptr::from_ref(session.artifact.as_ref().unwrap().artifact()).addr(),
        old_artifact
    );
    assert_eq!(
        session.compiled_project_cache.as_deref(),
        Some(&vec![1, 2, 3])
    );
    assert_eq!(
        session
            .full_project_file
            .as_ref()
            .map(|bytes| bytes.copy_range(0..bytes.len())),
        Some(vec![4, 5, 6])
    );
    assert_eq!(
        session
            .client_preferences
            .as_ref()
            .unwrap()
            .project_revision,
        1
    );

    submit(
        &mut session,
        4,
        RuntimeMessage::ProjectManifest(next_manifest(3)),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let successful_request = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("replacement image metadata request");
    submit(
        &mut session,
        5,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: successful_request,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 48,
                        height: 24,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(
        |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    ));
    let committed = session.project_snapshot.as_ref().unwrap();
    assert_eq!(committed.manifest.project_revision, 3);
    assert!(matches!(
        &committed.manifest.files[0].payload,
        FilePayload::Utf8(source) if source.is_empty() && source.capacity() == 0
    ));
    assert_eq!(
        committed
            .resource_graph
            .sprite("face")
            .map(|sprite| (sprite.width, sprite.height)),
        Some((48, 24))
    );
    assert!(session.compiled_project_cache.is_none());
    assert!(session.full_project_file.is_none());
    assert!(session.client_preferences.is_none());
}
