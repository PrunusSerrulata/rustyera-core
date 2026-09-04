#[test]
fn queued_input_is_processed_before_one_cooperative_cache_quantum() {
    for message_skip in [false, true] {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "cooperative-cache-input-test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
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
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nPRINTL first\nWAIT\nPRINTL accepted\nWAIT\nRETURN\n".into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let _ = drain(&mut session);
        submit(
            &mut session,
            2,
            RuntimeMessage::Start(StartRequest {
                mode: StartMode::NewGame { seed: Some(1) },
            }),
        );
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let pending = session.operations.active_input().unwrap();
        let wait_id = pending.wait.wait_id;
        let token = pending.wait.submission_token;
        let artifact = session.artifact.clone().unwrap();
        let snapshot = session.project_snapshot.as_ref().unwrap();
        let encoder = crate::compiled_cache::CooperativeCompiledCacheEncoder::new(
            Arc::clone(&snapshot.manifest),
            session.extension_declarations.clone(),
            artifact.clone(),
            session
                .incremental
                .compact_cache_keys(artifact.artifact())
                .unwrap(),
            crate::compiled_cache::CompiledSnapshotMetadata::from(snapshot),
            session.compiled_cache_diagnostics.clone(),
            None,
        );
        session.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
            encoder: Box::new(encoder),
        });
        submit(
            &mut session,
            3,
            RuntimeMessage::Input(FrontendInput {
                wait_id,
                token,
                monotonic_time_ns: 0,
                intent: InputIntent::Enter,
                message_skip,
            }),
        );

        let report = session.drive(RuntimeDriveBudget::default()).unwrap();
        let messages = drain(&mut session);

        assert!(report.runtime_transitions > 0);
        assert!(report.cooperative_background_work);
        assert!(session.compiled_cache_task.is_some());
        assert!(messages.iter().all(|message| !matches!(
            message,
            RuntimeMessage::CommandRejected(CommandRejected { .. })
        )));
    }
}

fn cooperative_cache_session() -> (RuntimeSession, ProjectManifest, ProjectIdentity) {
    cooperative_cache_session_with_options(RuntimeOptions::default())
}

fn low_memory_cooperative_cache_session() -> (RuntimeSession, ProjectManifest, ProjectIdentity) {
    cooperative_cache_session_with_options(RuntimeOptions {
        retain_project_source_payloads: false,
        ..RuntimeOptions::default()
    })
}

fn cooperative_cache_session_with_options(
    options: RuntimeOptions,
) -> (RuntimeSession, ProjectManifest, ProjectIdentity) {
    let manifest = ProjectManifest {
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
                relative_path: "resources/empty.bin".into(),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(Vec::new())),
                content_hash: None,
            },
        ],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(options);
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session
        .load_project(
            99,
            ProjectLoadRequest {
                identity: identity.clone(),
                manifest: Some(manifest.clone()),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let _ = drain(&mut session);
    (session, manifest, identity)
}

fn cooperative_cache_encoder(
    session: &RuntimeSession,
    manifest: Arc<ProjectManifest>,
) -> crate::compiled_cache::CooperativeCompiledCacheEncoder {
    let artifact = session.artifact.clone().unwrap();
    let snapshot = session.project_snapshot.as_ref().unwrap();
    crate::compiled_cache::CooperativeCompiledCacheEncoder::new(
        manifest,
        session.extension_declarations.clone(),
        artifact.clone(),
        session
            .incremental
            .compact_cache_keys(artifact.artifact())
            .unwrap(),
        crate::compiled_cache::CompiledSnapshotMetadata::from(snapshot),
        session.compiled_cache_diagnostics.clone(),
        None,
    )
}

fn finish_cooperative_cache_task(session: &mut RuntimeSession) -> Vec<RuntimeMessage> {
    let mut messages = Vec::new();
    for _ in 0..256 {
        if session.compiled_cache_task.is_none() {
            break;
        }
        assert!(session.poll_compiled_cache_task().unwrap());
        messages.extend(drain(session));
    }
    assert!(session.compiled_cache_task.is_none());
    messages
}

#[test]
fn cooperative_cache_task_publishes_one_ready_diagnostic() {
    let (mut session, _manifest, _identity) = cooperative_cache_session();

    let canonical_manifest = Arc::clone(&session.project_snapshot.as_ref().unwrap().manifest);
    session.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
        encoder: Box::new(cooperative_cache_encoder(&session, canonical_manifest)),
    });
    let completion = finish_cooperative_cache_task(&mut session);
    assert_eq!(
        completion
            .iter()
            .filter(|message| matches!(
                message,
                RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
                    if code == "runtime.compiled_cache_ready"
            ))
            .count(),
        1
    );
    assert!(!session.poll_compiled_cache_task().unwrap());
    assert!(drain(&mut session).is_empty());
}

#[test]
fn cooperative_cache_failure_is_unique_and_project_replacement_cancels_work() {
    let (mut session, manifest, identity) = cooperative_cache_session();

    let mut unreadable = manifest.clone();
    unreadable.files[1].payload = FilePayload::IoError(era_runtime_protocol::FrontendIoError {
        kind: FrontendIoErrorKind::Other,
        message: "fixture".into(),
        platform_code: None,
    });
    session.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
        encoder: Box::new(cooperative_cache_encoder(&session, Arc::new(unreadable))),
    });
    let failure = finish_cooperative_cache_task(&mut session);
    assert_eq!(
        failure
            .iter()
            .filter(|message| matches!(
                message,
                RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
                    if code == "runtime.compiled_cache_failed"
            ))
            .count(),
        1
    );

    session.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
        encoder: Box::new(cooperative_cache_encoder(
            &session,
            Arc::new(manifest.clone()),
        )),
    });
    let full_encoder = cooperative_cache_encoder(&session, Arc::new(manifest.clone()));
    session.full_project_task = Some(ProjectContainerTask::Cooperative {
        encoder: Box::new(full_encoder),
    });
    assert!(session.poll_compiled_cache_task().unwrap());
    session
        .load_project(
            100,
            ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    assert!(session.compiled_cache_task.is_none());
    assert!(session.full_project_task.is_none());
    assert!(drain(&mut session).iter().all(|message| !matches!(
        message,
        RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
            if code == "runtime.compiled_cache_ready" || code == "runtime.compiled_cache_failed"
    )));
}
