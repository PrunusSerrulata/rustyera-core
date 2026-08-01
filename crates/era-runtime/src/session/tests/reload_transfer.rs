use super::*;

#[test]
#[allow(clippy::too_many_lines)]
fn project_resource_metadata_requests_respect_pending_request_backpressure() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.services.push(ServiceCapability {
        kind: ServiceKind::Image,
        operation: IMAGE_METADATA_OPERATION.into(),
        versions: VersionRange::exact(IMAGE_METADATA_OPERATION_VERSION),
    });
    let mut requested_limits = RuntimeOptions::default().limits;
    requested_limits.maximum_pending_requests = 2;
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "resource-backpressure-test".into(),
            features: Vec::new(),
            requested_limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let _ = drain(&mut session);
    let mut files = vec![
        SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        },
        SubmittedFile {
            relative_path: "resources/sprites.csv".into(),
            category: FileCategory::ResourceManifest,
            payload: FilePayload::Utf8("A,a.png\nB,b.png\nC,c.png".into()),
            content_hash: None,
        },
    ];
    files.extend(
        ["a.png", "b.png", "c.png"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| SubmittedFile {
                relative_path: format!("resources/{name}"),
                category: FileCategory::Resource,
                payload: FilePayload::Bytes(ProtocolBytes::new(vec![u8::try_from(index).unwrap()])),
                content_hash: None,
            }),
    );
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let first = drain(&mut session);
    let mut outstanding = first
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == IMAGE_METADATA_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(outstanding.len(), 2);

    submit(
        &mut session,
        2,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: outstanding.remove(0),
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 1,
                        height: 1,
                        format: "png".into(),
                        animated: false,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let second = drain(&mut session);
    outstanding.extend(second.iter().filter_map(|message| match message {
        RuntimeMessage::ServiceRequest(request)
            if request.operation == IMAGE_METADATA_OPERATION =>
        {
            Some(request.request_id)
        }
        _ => None,
    }));
    assert_eq!(outstanding.len(), 2);

    for (offset, request_id) in outstanding.into_iter().enumerate() {
        submit(
            &mut session,
            3 + offset as u64,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&ImageMetadataResponse {
                            width: 1,
                            height: 1,
                            format: "png".into(),
                            animated: false,
                        })
                        .unwrap(),
                    ),
                },
            }),
        );
    }
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
}

#[test]
fn font_profile_is_session_fixed_case_insensitive_and_deterministic() {
    let mut requested = capabilities();
    requested.available_fonts = vec!["Zeta".into(), "alpha".into(), "ALPHA".into()];
    requested.font_metrics = true;
    requested.services.push(ServiceCapability {
        kind: ServiceKind::FontMetrics,
        operation: GGET_TEXT_SIZE_OPERATION.into(),
        versions: VersionRange::exact(GGET_TEXT_SIZE_OPERATION_VERSION),
    });
    let selected = selected_capabilities(&requested);
    assert_eq!(selected.available_fonts, vec!["alpha", "Zeta"]);
    assert!(selected.font_metrics);
}

#[test]
fn effect_acknowledgements_are_exact_and_failures_become_diagnostics() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.epoch = SessionEpoch(1);
    session
        .emit_effect(EffectKind::StartAnimation("flash".into()))
        .expect("emit effect");
    let _ = drain(&mut session);
    session
        .handle_message(
            10,
            RuntimeMessage::EffectAcknowledgement(EffectAcknowledgement {
                outcomes: vec![era_runtime_protocol::EffectOutcome {
                    effect_id: 1,
                    status: EffectOutcomeStatus::Failed,
                    message: Some("device unavailable".into()),
                }],
            }),
        )
        .expect("acknowledge effect");
    assert!(session.effect_journal.is_empty());
    assert!(matches!(
        drain(&mut session).as_slice(),
        [RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })]
            if code == "runtime.device_effect_failed"
    ));
}

#[test]
fn return_to_title_reuses_the_loaded_artifact_without_project_loading() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nWAIT\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let mut build = crate::project::build_project(&manifest, None);
    assert!(build.report.success, "{:?}", build.report.diagnostics);
    let expected = build
        .artifact
        .as_ref()
        .unwrap()
        .artifact()
        .manifest
        .artifact_id;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session.artifact = build.artifact.take();
    session.incremental = build.incremental;
    session.project_snapshot = build.snapshot;
    session.start_new_game(7).unwrap();

    assert!(std::ptr::eq(
        session.artifact.as_ref().unwrap().artifact(),
        session.vm.as_ref().unwrap().vm().artifact(),
    ));

    session.return_to_title(99).unwrap();

    assert_eq!(session.phase, RuntimePhase::Starting);
    assert_eq!(
        session
            .artifact
            .as_ref()
            .unwrap()
            .artifact()
            .manifest
            .artifact_id,
        expected
    );
    assert!(
        drain(&mut session)
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
}

#[test]
fn compiled_cache_export_prepares_the_payload_off_thread() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);

    session
        .load_project(
            99,
            &ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();

    assert!(session.compiled_project_cache.is_none());
    assert!(session.compiled_cache_task.is_none());
    let _ = drain(&mut session);
    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(session.compiled_cache_task.is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message == "compiled project cache preparation started"
    )));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.compiled_cache_task.is_some() {
        session.poll_compiled_cache_task().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "compiled cache worker did not finish"
        );
        std::thread::yield_now();
    }
    let completion = drain(&mut session);
    assert!(
        completion.iter().any(|message| matches!(
            message,
            RuntimeMessage::Diagnostic(ProtocolDiagnostic { code, .. })
                if code == "runtime.compiled_cache_ready"
        )),
        "{completion:#?}"
    );
    let bytes = session.compiled_project_cache.as_ref().unwrap();
    assert!(crate::compiled_cache::decode(bytes, 64 * 1024 * 1024).is_ok());

    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportReady(StateExportReady {
            kind: StateExportKind::CompiledProjectCache,
            result: StateExportResult::Ready { .. },
        })
    )));
}

#[test]
fn compiled_cache_export_does_not_retry_a_failed_background_build() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.compiled_cache_failure = Some("synthetic encoding failure".into());

    session
        .export_state(
            100,
            StateExportRequest {
                kind: StateExportKind::CompiledProjectCache,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();

    assert!(session.compiled_cache_task.is_none());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::ResourceLimit,
            message,
            ..
        }) if message.contains("synthetic encoding failure")
    )));
}

#[test]
fn project_load_rejects_an_uncommitted_cache_without_changing_phase() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);

    session
        .load_project(
            99,
            &ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: 1,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: Some(ProjectManifest {
                    project_revision: 1,
                    files: Vec::new(),
                }),
                compiled_cache_transfer_id: Some(123),
            },
        )
        .unwrap();

    assert_eq!(session.phase, RuntimePhase::Ready);
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));
}

#[test]
fn identity_only_project_load_requests_payload_after_a_cache_miss() {
    let manifest = ProjectManifest {
        project_revision: 4,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let session = RuntimeSession::new(RuntimeOptions::default());

    let Err(report) = session.build_project_from_cache(
        &ProjectLoadRequest {
            identity,
            manifest: None,
            compiled_cache_transfer_id: None,
        },
        None,
    ) else {
        panic!("an identity without an exact cache needs source payloads");
    };

    assert!(!report.success);
    assert!(report.payload_required);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_payload_required")
    );
}

#[test]
fn exact_compiled_cache_load_does_not_require_a_manifest() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    let mut initial = crate::project::build_project(&manifest, None);
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    initial.report.diagnostics.push(ProtocolDiagnostic {
        code: "compiler.cached_warning".into(),
        level: RuntimeLogLevel::Warning,
        message: "warning retained with compiled output".into(),
        source: Some(era_runtime_protocol::SourceLocation {
            relative_path: "main.erb".into(),
            byte_start: 0,
            byte_end: 13,
            line: Some(0),
            byte_column: Some(0),
        }),
    });
    initial.incremental.compact();
    let cache = crate::compiled_cache::encode(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
        &initial.report.diagnostics,
    )
    .unwrap();
    let mut identity = crate::compiled_cache::project_identity(&manifest);
    identity.project_revision = 8;
    let session = RuntimeSession::new(RuntimeOptions::default());

    let cached = session
        .build_project_from_cache(
            &ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
        )
        .expect("an exact cache loads from source identity alone");

    assert!(cached.report.success);
    assert!(!cached.report.payload_required);
    assert_eq!(cached.report.project_revision, 8);
    let replayed = cached
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "compiler.cached_warning")
        .expect("cached compiler warning is replayed");
    assert_eq!(replayed.level, RuntimeLogLevel::Warning);
    assert_eq!(
        replayed.message,
        "[cached] warning retained with compiled output"
    );
    assert_eq!(replayed.source.as_ref().unwrap().byte_end, 13);
    assert_eq!(cached.snapshot.unwrap().manifest.project_revision, 8);
}
