#[test]
fn ready_project_reload_stages_and_commits_a_normalized_delta() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "reload-test".into(),
            features: vec![RuntimeFeature::ProjectReload],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![FileChange::Upsert {
                file: SubmittedFile {
                    relative_path: "./main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL reloaded\nRETURN\n".into()),
                    content_hash: None,
                },
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert_eq!(session.phase(), RuntimePhase::Ready);
    assert_eq!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .manifest
            .project_revision,
        2
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(era_runtime_protocol::ProjectLoadReport {
            project_revision: 2,
            success: true,
            ..
        })
    )));
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "runtime.input_undo_invalidated"
    )));
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["kind"], "hot_reload");
    assert_eq!(replay_header["origin"]["before_revision"], "1");
    assert_eq!(replay_header["origin"]["after_revision"], "2");
    assert_eq!(replay_header["step_count"], 0);

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 2,
            target_revision: 3,
            changes: Vec::new(),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let unchanged_replay = input_replay_records(&session).remove(0);
    assert_eq!(unchanged_replay["origin"], replay_header["origin"]);
}

#[test]
#[allow(clippy::too_many_lines)]
fn incompatible_low_memory_hot_reload_keeps_the_sparse_live_project() {
    let mut session = RuntimeSession::new(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    });
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "reload-transaction-test".into(),
            features: vec![RuntimeFeature::ProjectReload],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let script = "@SYSTEM_TITLE\nWAIT\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "globals.erh".into(),
                    category: FileCategory::Erh,
                    payload: FilePayload::Utf8("#DIM GLOBAL_VALUE\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(script.into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    let before = session.project_snapshot.as_ref().unwrap();
    let before_identity = before.project_identity;
    let before_payloads = before
        .manifest
        .files
        .iter()
        .map(|file| file.payload.clone())
        .collect::<Vec<_>>();
    let before_artifact = std::ptr::from_ref(session.vm.as_ref().unwrap().vm().artifact()).addr();

    submit(
        &mut session,
        3,
        RuntimeMessage::ReloadProject(ReloadProject {
            base_revision: 1,
            target_revision: 2,
            changes: vec![
                FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "globals.erh".into(),
                        category: FileCategory::Erh,
                        payload: FilePayload::Utf8("#DIMS GLOBAL_VALUE\n".into()),
                        content_hash: None,
                    },
                },
                FileChange::Upsert {
                    file: SubmittedFile {
                        relative_path: "main.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(script.into()),
                        content_hash: None,
                    },
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report)
            if !report.success
                && report.diagnostics.iter().any(|diagnostic| diagnostic.code == "runtime.hot_reload_incompatible")
    )));
    let retained = session.project_snapshot.as_ref().unwrap();
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert_eq!(retained.manifest.project_revision, 1);
    assert_eq!(retained.project_identity, before_identity);
    assert_eq!(
        retained
            .manifest
            .files
            .iter()
            .map(|file| file.payload.clone())
            .collect::<Vec<_>>(),
        before_payloads
    );
    assert_eq!(
        std::ptr::from_ref(session.vm.as_ref().unwrap().vm().artifact()).addr(),
        before_artifact
    );
}

#[test]
fn state_import_rejects_out_of_order_chunks_and_bad_digests() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TraditionalSave],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);

    submit(
        &mut session,
        1,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: 3,
            digest: Some(ProtocolBytes::new([0; 32])),
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(accepted) => Some(accepted.transfer_id),
            _ => None,
        })
        .unwrap();

    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 1,
            data: ProtocolBytes::new([b'a']),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(*b"abc"),
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit {
            transfer_id,
            digest: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn full_project_manifest_import_accepts_commit_digest() {
    let mut session = negotiated_session();
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 7,
        files: Vec::new(),
    };
    let bytes = encode_canonical(&manifest).unwrap();
    submit(
        &mut session,
        1,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::FullProjectManifest,
            total_bytes: bytes.len() as u64,
            digest: None,
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(value) => Some(value.transfer_id),
            _ => None,
        })
        .unwrap();
    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(bytes[..2].to_vec()),
        }),
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 2,
            data: ProtocolBytes::new(bytes[2..].to_vec()),
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::StateImportCommit(StateImportCommit {
            transfer_id,
            digest: Some(ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec())),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateImportReady(StateImportReady {
            kind: StateExportKind::FullProjectManifest,
            ..
        })
    )));
    let staged = session.staged_full_project_manifest.as_ref().unwrap();
    assert_eq!(staged.manifest, manifest);
    assert_eq!(staged.source_transfer_id, Some(transfer_id));

    submit(
        &mut session,
        5,
        RuntimeMessage::StateTransferCancel(StateTransferCancel { transfer_id }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(
        drain(&mut session)
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::CommandRejected(_)))
    );
    assert!(session.staged_full_project_manifest.is_none());
}

#[test]
fn state_import_begin_enforces_digest_placement() {
    let mut session = negotiated_session();
    submit(
        &mut session,
        1,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::FullProjectManifest,
            total_bytes: 1,
            digest: Some(ProtocolBytes::new([0; 32])),
            artifact_id: None,
        }),
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::TraditionalSave,
            total_bytes: 1,
            digest: None,
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(
        drain(&mut session)
            .iter()
            .filter(|message| matches!(message, RuntimeMessage::CommandRejected(_)))
            .count(),
        2
    );
}

#[test]
fn state_import_commit_enforces_digest_placement() {
    let mut session = negotiated_session();
    let transfer_id = begin_state_import(
        &mut session,
        1,
        StateExportKind::FullProjectManifest,
        1,
        None,
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new([0xff]),
        }),
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportCommit(StateImportCommit {
            transfer_id,
            digest: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    let mut session = negotiated_session();
    let ordinary = [b'x'];
    let transfer_id = begin_state_import(
        &mut session,
        1,
        StateExportKind::TraditionalSave,
        1,
        Some(ProtocolBytes::new(
            blake3::hash(&ordinary).as_bytes().to_vec(),
        )),
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(ordinary),
        }),
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportCommit(StateImportCommit {
            transfer_id,
            digest: Some(ProtocolBytes::new(
                blake3::hash(&ordinary).as_bytes().to_vec(),
            )),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn full_project_manifest_import_cleans_up_malformed_cbor() {
    let mut session = negotiated_session();
    let malformed = [0xff];
    let transfer_id = begin_state_import(
        &mut session,
        1,
        StateExportKind::FullProjectManifest,
        malformed.len() as u64,
        None,
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(malformed),
        }),
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportCommit(StateImportCommit {
            transfer_id,
            digest: Some(ProtocolBytes::new(
                blake3::hash(&malformed).as_bytes().to_vec(),
            )),
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
    assert!(session.inbound_transfer.is_none());
}

#[test]
fn full_project_manifest_busy_commit_is_retryable() {
    let mut session = negotiated_session();
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 9,
        files: Vec::new(),
    };
    let bytes = encode_canonical(&manifest).unwrap();
    submit(
        &mut session,
        1,
        RuntimeMessage::StateImportBegin(StateImportBegin {
            kind: StateExportKind::FullProjectManifest,
            total_bytes: bytes.len() as u64,
            digest: None,
            artifact_id: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let transfer_id = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateImportAccepted(value) => Some(value.transfer_id),
            _ => None,
        })
        .unwrap();
    submit(
        &mut session,
        2,
        RuntimeMessage::StateImportChunk(StateImportChunk {
            transfer_id,
            offset: 0,
            data: ProtocolBytes::new(bytes.clone()),
        }),
    );
    session.staged_full_project_manifest = Some(StagedFullProjectManifest {
        source_transfer_id: None,
        manifest: ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 8,
            files: Vec::new(),
        },
    });
    let commit = StateImportCommit {
        transfer_id,
        digest: Some(ProtocolBytes::new(blake3::hash(&bytes).as_bytes().to_vec())),
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::StateImportCommit(commit.clone()),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidState,
            ..
        })
    )));
    assert!(session.inbound_transfer.is_some());

    session.staged_full_project_manifest = None;
    submit(&mut session, 4, RuntimeMessage::StateImportCommit(commit));
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateImportReady(StateImportReady {
            kind: StateExportKind::FullProjectManifest,
            ..
        })
    )));
    assert_eq!(
        session
            .staged_full_project_manifest
            .as_ref()
            .unwrap()
            .manifest,
        manifest
    );
}

#[test]
fn host_staged_compiled_cache_reuses_the_owned_payload() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let payload = vec![7; 4096];

    let transfer_id = session
        .stage_compiled_project_cache(payload.clone())
        .expect("host cache staging should accept an in-limit payload");
    let staged = session
        .consume_state_import(1, transfer_id, StateExportKind::CompiledProjectCache)
        .unwrap()
        .expect("staged cache should be committed immediately");

    assert_eq!(staged, payload);
    assert!(session.inbound_transfer.is_none());
}
