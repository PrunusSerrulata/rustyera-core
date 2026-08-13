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
            configuration_profile: None,
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
    session.incremental = Arc::new(build.incremental);
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
    let messages = drain(&mut session);
    assert!(
        messages
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
    let entropy_request = messages
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == RANDOM_SEED_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("return-to-title entropy request");
    session
        .handle_message(
            100,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: entropy_request,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&RandomSeedResponse { seed: 9 }).unwrap(),
                    ),
                },
            }),
        )
        .unwrap();
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["kind"], "new_game");
    assert_eq!(replay_header["origin"]["seed"], "9");
    assert_eq!(replay_header["origin"]["trigger"], "return_to_title");
}

#[test]
fn compiled_cache_export_prepares_the_payload_off_thread() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "emuera.config".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8("Font size:18\n".into()),
                content_hash: None,
            },
        ],
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

    let generated = session
        .project_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.generated_configuration_source.as_deref())
        .expect("legacy configuration generates reraconfig.toml");
    assert_rera_font_size(generated, 18);

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
    let decoded = crate::compiled_cache::decode(bytes, 64 * 1024 * 1024).unwrap();
    assert_manifest_rera_font_size(&decoded.snapshot.manifest, 18);
    assert!(crate::compiled_cache::decode_project_file(bytes, bytes.len()).is_err());

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
fn full_project_export_preempts_cache_streams_chunks_and_cancels_cleanly() {
    let (mut session, manifest, _) = cooperative_cache_session();
    let cache_manifest = Arc::clone(&session.project_snapshot.as_ref().unwrap().manifest);
    session.compiled_cache_task = Some(ProjectContainerTask::Cooperative {
        encoder: Box::new(cooperative_cache_encoder(&session, cache_manifest)),
    });
    let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&progress);
    session.project_progress_reporter = Some(ProjectProgressReporter::new(move |value| {
        observed.lock().unwrap().push(value);
    }));

    session
        .stage_full_project_manifest(
            100,
            FullProjectManifest {
                manifest: manifest.clone(),
            },
        )
        .unwrap();
    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    assert!(session.compiled_cache_task.is_none());
    assert!(session.full_project_task.is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message == "full project preparation started"
    )));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.full_project_task.is_some() {
        session.poll_full_project_task();
        assert!(
            std::time::Instant::now() < deadline,
            "full project worker did not finish"
        );
        std::thread::yield_now();
    }
    let reports = progress.lock().unwrap();
    assert!(reports.iter().any(|value| {
        value.stage == ProjectProgressStage::Packaging && value.total > 1 && value.completed > 0
    }));
    drop(reports);

    session
        .export_state(
            102,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();
    let ready = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ready { transfer },
                ..
            }) => Some(transfer),
            _ => None,
        })
        .expect("full project transfer is ready");
    session
        .read_state_export(
            103,
            StateExportChunkRequest {
                transfer_id: ready.transfer_id,
                offset: 0,
                maximum_bytes: 17,
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::StateExportChunk(StateExportChunk { offset: 0, .. })
    )));
    session.cancel_state_export(StateExportCancel {
        kind: StateExportKind::FullProjectFile,
    });
    assert!(session.outbound_transfer.is_none());
    assert!(session.full_project_task.is_none());
    assert!(session.staged_full_project_manifest.is_none());
}

#[test]
fn full_project_export_rejects_a_stale_materialized_manifest() {
    let (mut session, mut manifest, _) = cooperative_cache_session();
    manifest.files[0].payload = FilePayload::Utf8("@SYSTEM_TITLE\nPRINTL changed\nRETURN\n".into());
    session
        .stage_full_project_manifest(100, FullProjectManifest { manifest })
        .unwrap();
    session
        .export_state(
            101,
            StateExportRequest {
                kind: StateExportKind::FullProjectFile,
                snapshot_purpose: SnapshotExportPurpose::Normal,
            },
        )
        .unwrap();

    assert!(session.full_project_task.is_none());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected { message, .. })
            if message.contains("changed after the active project")
    )));
}

fn assert_manifest_rera_font_size(manifest: &ProjectManifest, expected: i64) {
    let source = manifest
        .files
        .iter()
        .find(|file| file.relative_path.eq_ignore_ascii_case("reraconfig.toml"))
        .and_then(|file| match &file.payload {
            FilePayload::Utf8(source) => Some(source.as_str()),
            _ => None,
        })
        .expect("compiled project embeds the generated reraconfig.toml");
    assert_rera_font_size(source, expected);
}

fn assert_rera_font_size(source: &str, expected: i64) {
    let values = era_config::ReraConfigDocument::parse(source)
        .unwrap()
        .values()
        .unwrap();
    assert_eq!(
        values.get_code("FontSize"),
        Some(&era_config::ConfigValue::Integer(expected))
    );
}

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
    let manifest = ProjectManifest {
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
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session
        .load_project(
            99,
            &ProjectLoadRequest {
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
            &ProjectLoadRequest {
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
fn host_staged_manifest_is_owned_busy_single_use_and_identity_checked() {
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
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Ready;

    session.stage_project_manifest(manifest.clone()).unwrap();
    assert!(matches!(
        session.stage_project_manifest(manifest.clone()),
        Err(RuntimeError::Busy(_))
    ));
    session
        .load_project(
            41,
            &ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let report = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(report.success, "{:?}", report.diagnostics);
    assert!(session.staged_project_manifest.is_none());

    session
        .load_project(
            42,
            &ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let missing = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(missing.payload_required);

    session.stage_project_manifest(manifest).unwrap();
    session
        .load_project(
            43,
            &ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: identity.project_revision,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    let mismatch = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(
        mismatch
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_identity_mismatch")
    );
    assert!(session.staged_project_manifest.is_none());
}

#[test]
fn rejected_or_explicit_project_load_discards_a_host_staged_manifest() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: Vec::new(),
    };
    let identity = crate::compiled_cache::project_identity(&manifest);
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Running;
    session.stage_project_manifest(manifest.clone()).unwrap();

    session
        .load_project(
            51,
            &ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    assert!(session.staged_project_manifest.is_none());

    session.phase = RuntimePhase::Ready;
    session.stage_project_manifest(manifest.clone()).unwrap();
    session
        .load_project(
            52,
            &ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    assert!(session.staged_project_manifest.is_none());
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
    let cache = crate::compiled_cache::encode_compiled_cache_for_test(
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

#[test]
#[allow(clippy::too_many_lines)]
fn host_staged_exact_cache_uses_the_normal_project_load_contract() {
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
    initial.incremental.compact();
    let cache = crate::compiled_cache::encode_compiled_cache_for_test(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
        &initial.report.diagnostics,
    )
    .unwrap();
    let identity = crate::compiled_cache::project_identity(&manifest);
    let expected_session = RuntimeSession::new(RuntimeOptions::default());
    let expected = expected_session
        .build_project_from_cache(
            &ProjectLoadRequest {
                identity: identity.clone(),
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
        )
        .unwrap()
        .report;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Ready;
    let mismatch_cache = cache.clone();
    session.stage_project_manifest(manifest.clone()).unwrap();
    let transfer_id = session.stage_compiled_project_cache(cache).unwrap();

    session
        .load_project(
            44,
            &ProjectLoadRequest {
                identity,
                manifest: None,
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();

    let mut report = None;
    while let Some(bytes) = session.poll_envelope() {
        let envelope = decode_envelope(&bytes, WireLimits::default()).unwrap();
        let message = RuntimeMessage::from_envelope(&envelope).unwrap();
        assert!(!matches!(
            message,
            RuntimeMessage::StateImportAccepted(_) | RuntimeMessage::StateImportReady(_)
        ));
        if let RuntimeMessage::ProjectLoadReport(value) = message {
            assert_eq!(envelope.correlation_id, Some(44));
            report = Some(value);
        }
    }
    let report = report.expect("staged cache load emits a project report");
    assert_eq!(report.success, expected.success);
    assert_eq!(report.payload_required, expected.payload_required);
    assert_eq!(report.project_revision, expected.project_revision);
    assert_eq!(report.diagnostics, expected.diagnostics);
    assert_eq!(session.phase, RuntimePhase::Ready);
    assert!(session.inbound_transfer.is_none());
    assert!(session.staged_project_manifest.is_none());

    session
        .load_project(
            45,
            &ProjectLoadRequest {
                identity: crate::compiled_cache::project_identity(&manifest),
                manifest: None,
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(CommandRejected {
            code: CommandErrorCode::InvalidValue,
            ..
        })
    )));

    let mut mismatch = RuntimeSession::new(RuntimeOptions::default());
    mismatch.phase = RuntimePhase::Ready;
    let transfer_id = mismatch
        .stage_compiled_project_cache(mismatch_cache)
        .unwrap();
    mismatch
        .load_project(
            46,
            &ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: manifest.project_revision,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: Some(manifest),
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();
    let mismatch_report = drain(&mut mismatch)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(!mismatch_report.success);
    assert!(!mismatch_report.payload_required);
    assert!(
        mismatch_report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_identity_mismatch")
    );
    assert!(mismatch.inbound_transfer.is_none());
}

#[test]
fn host_staged_corrupt_cache_reports_a_normal_cache_miss() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.phase = RuntimePhase::Ready;
    let transfer_id = session.stage_compiled_project_cache(vec![7; 64]).unwrap();

    session
        .load_project(
            51,
            &ProjectLoadRequest {
                identity: ProjectIdentity {
                    project_revision: 9,
                    source_digest: ProtocolBytes::new(vec![0; 32]),
                },
                manifest: None,
                compiled_cache_transfer_id: Some(transfer_id),
            },
        )
        .unwrap();

    let report = drain(&mut session)
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ProjectLoadReport(report) => Some(report),
            _ => None,
        })
        .unwrap();
    assert!(!report.success);
    assert!(report.payload_required);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_ignored")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.project_payload_required")
    );
    assert!(session.inbound_transfer.is_none());
}

#[test]
fn compiled_cache_is_reused_across_configuration_profiles() {
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    for (producer, consumer) in [
        (ConfigurationClientProfile::Tui, ConfigurationClientProfile::Browser),
        (ConfigurationClientProfile::Browser, ConfigurationClientProfile::Tui),
    ] {
        let mut initial = crate::project::build_project_with_extensions_and_progress(
            &manifest, None, None, &[], producer, None,
        );
        assert!(initial.report.success, "{:?}", initial.report.diagnostics);
        initial.incremental.compact();
        let cache = crate::compiled_cache::encode_compiled_cache_for_test(
            &manifest,
            &[],
            initial.artifact.as_ref().unwrap(),
            &initial.incremental,
            initial.snapshot.as_ref().unwrap(),
            &initial.report.diagnostics,
        )
        .unwrap();
        let mut identity = crate::compiled_cache::project_identity(&manifest);
        identity.project_revision = 9;
        let mut compact_manifest = manifest.clone();
        compact_manifest.project_revision = 9;
        compact_manifest.files[0].content_hash = Some(ProtocolBytes::new(
            blake3::hash(b"@SYSTEM_TITLE\nRETURN\n").as_bytes().to_vec(),
        ));
        compact_manifest.files[0].payload = FilePayload::Utf8(String::new());
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        session.configuration_profile = consumer;

        let build = session
            .build_project_from_cache(
                &ProjectLoadRequest {
                    identity,
                    manifest: Some(compact_manifest),
                    compiled_cache_transfer_id: None,
                },
                Some(&cache),
            )
            .expect("a host-neutral compiled cache should load in either frontend profile");

        assert!(build.report.success);
        assert!(!build.report.payload_required);
        assert_eq!(build.report.project_revision, 9);
        assert!(build
            .report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "runtime.compiled_cache_hit"));
        assert_eq!(build.snapshot.unwrap().configuration_profile, consumer);
    }
}

#[test]
fn journaled_configuration_rebuilds_instead_of_exact_hitting_the_old_artifact() {
    let old_configuration = "[audio]\nvolume = 100\n";
    let manifest = ProjectManifest {
        project_revision: 1,
        files: vec![
            SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            },
            SubmittedFile {
                relative_path: "reraconfig.toml".into(),
                category: FileCategory::Configuration,
                payload: FilePayload::Utf8(old_configuration.into()),
                content_hash: None,
            },
        ],
    };
    let mut initial = crate::project::build_project(&manifest, None);
    assert!(initial.report.success, "{:?}", initial.report.diagnostics);
    initial.incremental.compact();
    let mut cache = crate::compiled_cache::encode_full_project_for_test(
        &manifest,
        &[],
        initial.artifact.as_ref().unwrap(),
        &initial.incremental,
        initial.snapshot.as_ref().unwrap(),
        &initial.report.diagnostics,
    )
    .unwrap();
    let old_key = crate::compiled_cache::decode(&cache, cache.len())
        .unwrap()
        .key;
    let expected = blake3::hash(old_configuration.as_bytes());
    let update = crate::compiled_cache::prepare_project_configuration_update(
        &cache,
        usize::MAX,
        expected.as_bytes(),
        "[audio]\nvolume = 42\n",
    )
    .unwrap();
    cache.extend_from_slice(&update.append);
    let request_identity = crate::compiled_cache::decode_project_file(&cache, cache.len())
        .unwrap()
        .identity;
    let expected_key = crate::compiled_cache::project_key(&request_identity, &[]);
    assert_ne!(old_key, expected_key);

    let session = RuntimeSession::new(RuntimeOptions::default());
    let rebuilt = session
        .build_project_from_cache(
            &ProjectLoadRequest {
                identity: request_identity,
                manifest: None,
                compiled_cache_transfer_id: None,
            },
            Some(&cache),
        )
        .expect("journaled embedded manifest can safely rebuild the project");

    assert!(rebuilt.report.success, "{:?}", rebuilt.report.diagnostics);
    assert!(
        rebuilt
            .report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "runtime.compiled_cache_hit")
    );
    assert!(
        rebuilt
            .snapshot
            .unwrap()
            .manifest
            .files
            .iter()
            .any(|file| matches!(
                &file.payload,
                FilePayload::Utf8(source) if source == "[audio]\nvolume = 42\n"
            ))
    );
}
