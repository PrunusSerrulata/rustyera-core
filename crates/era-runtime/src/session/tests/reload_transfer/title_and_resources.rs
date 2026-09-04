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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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

fn title_session_fixture() -> (
    RuntimeSession,
    Arc<std::sync::Mutex<Vec<ProjectProgress>>>,
    ProjectManifest,
    erabasic_bytecode::Digest,
) {
    let manifest = ProjectManifest {
        compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
        project_revision: 1,
        files: vec![SubmittedFile {
            relative_path: "main.erb".into(),
            category: FileCategory::Erb,
            payload: FilePayload::Utf8("@SYSTEM_TITLE\nWAIT\nRETURN\n".into()),
            content_hash: None,
        }],
    };
    title_session_fixture_with_manifest(manifest, 7)
}

fn title_session_fixture_with_manifest(
    manifest: ProjectManifest,
    seed: u64,
) -> (
    RuntimeSession,
    Arc<std::sync::Mutex<Vec<ProjectProgress>>>,
    ProjectManifest,
    erabasic_bytecode::Digest,
) {
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
    let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = Arc::clone(&progress);
    session.project_progress_reporter = Some(ProjectProgressReporter::new(move |value| {
        observed.lock().unwrap().push(value);
    }));
    session.start_new_game(seed).unwrap();
    let _ = drain(&mut session);
    (session, progress, manifest, expected)
}

fn return_to_title_entropy_request(messages: Vec<RuntimeMessage>) -> u64 {
    messages
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == RANDOM_SEED_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("return-to-title entropy request")
}

fn answer_entropy(session: &mut RuntimeSession, request_id: u64, seed: u64) {
    session
        .handle_message(
            100,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id,
                result: ServiceResult::Ready {
                    payload: ProtocolBytes::new(
                        encode_canonical(&RandomSeedResponse { seed }).unwrap(),
                    ),
                },
            }),
        )
        .unwrap();
}

fn seed_title_timeline_retirement_state(session: &mut RuntimeSession) {
    session
        .presentation
        .append_text("old timeline sentinel".into(), false);
    session.pending_presentation_update = true;
    session
        .operations
        .insert_service(77, PendingService::StartEntropy);
    let project = session.project_snapshot.as_mut().unwrap();
    assert_eq!(project.resource_graph.create_canvas(17, 64, 64), Ok(true));
    assert!(
        project
            .resource_graph
            .draw_canvas_text(17, "retained canvas command".into(), [0, 0])
    );
    session
        .presentation
        .set_resource_replay(project.resource_graph.replay());
    session.hotkey_state = vec![1];
    session.text_box = "retained text box".into();
    session.flow_input_default_string = "retained input".into();
    session.debug_output = "retained debug output".into();
    session
        .save_extensions
        .push(era_runtime_save::OpaqueSaveExtension {
            type_tag: 0x7f,
            key: "retained".into(),
            payload: vec![1],
        });
    session.load_slot_paths.push("save01.sav".into());
    session
        .slot_labels
        .insert("save01.sav".into(), "slot".into());
    session.last_projection_state = Some(ProjectionState {
        runtime_revision: 1,
        text_box: "retained projection".into(),
        hotkey_state: vec![1],
        button_generation: 1,
        text_box_layout: TextBoxLayout::default(),
    });
    session.logical_time_ns = 123_456;
    session.frontend_time_origin = Some((7, 123_456));
}

fn assert_title_timeline_retired(session: &RuntimeSession) {
    assert_eq!(session.phase, RuntimePhase::Starting);
    assert!(session.vm.is_none());
    assert!(session.retained_title_program.is_some());
    assert!(session.hotkey_state.is_empty());
    assert!(session.text_box.is_empty());
    assert!(session.flow_input_default_string.is_empty());
    assert!(session.debug_output.is_empty());
    assert!(session.save_extensions.is_empty());
    assert!(session.load_slot_paths.is_empty());
    assert!(session.slot_labels.is_empty());
    assert!(session.last_projection_state.is_none());
    assert_eq!(session.logical_time_ns, 123_456);
    assert!(session.frontend_time_origin.is_none());
    assert!(
        session
            .project_snapshot
            .as_ref()
            .unwrap()
            .resource_graph
            .canvas_state(17)
            .is_none()
    );
}

#[test]
fn return_to_title_reuses_program_index_without_project_loading() {
    let (mut session, progress, _manifest, expected) = title_session_fixture();

    assert!(std::ptr::eq(
        session.artifact.as_ref().unwrap().artifact(),
        session.vm.as_ref().unwrap().vm().artifact(),
    ));
    assert!(
        progress
            .lock()
            .unwrap()
            .iter()
            .any(|value| value.stage == ProjectProgressStage::IndexingProgram)
    );
    progress.lock().unwrap().clear();

    session.return_to_title(99).unwrap();
    assert!(session.vm.is_none());
    assert!(session.retained_title_program.is_some());
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
    answer_entropy(&mut session, return_to_title_entropy_request(messages), 9);

    assert!(session.retained_title_program.is_none());
    assert!(session.vm.is_some());
    let resumed_progress = progress.lock().unwrap();
    assert!(
        resumed_progress
            .iter()
            .any(|value| value.stage == ProjectProgressStage::InitializingMemory)
    );
    assert!(
        resumed_progress
            .iter()
            .all(|value| value.stage != ProjectProgressStage::IndexingProgram)
    );
    drop(resumed_progress);
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["kind"], "new_game");
    assert_eq!(replay_header["origin"]["seed"], "9");
    assert_eq!(replay_header["origin"]["trigger"], "return_to_title");
}

#[test]
#[allow(clippy::too_many_lines)]
fn als_only_reload_keeps_old_frames_and_restarts_title_with_updated_alias() {
    let file = |path: &str, category, contents: &str| SubmittedFile {
        relative_path: path.into(),
        category,
        payload: FilePayload::Utf8(contents.into()),
        content_hash: None,
    };
    let manifest = ProjectManifest {
        compatibility: erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        project_revision: 1,
        files: vec![
            file(
                "reraconfig.toml",
                FileCategory::Configuration,
                "[meta]\nschema_version = 4\n[compatibility]\nprofile = \"emuera.skia.snake\"\n",
            ),
            file("ERB/definitions.erh", FileCategory::Erh, "#DIM BUFF,32\n"),
            file(
                "ERB/main.erb",
                FileCategory::Erb,
                "@SYSTEM_TITLE\nBUFF:main = 42\nBUFF:other = 84\nWHILE 1\nCALL REPORT_ALIAS\nINPUT\nWEND\nRETURN\n\n@REPORT_ALIAS\nPRINTFORML ALIAS={BUFF:alias}\nRETURN\n",
            ),
            file(
                "ERB/indices/BUFF.erd",
                FileCategory::Erd,
                "10,main\n11,other\n",
            ),
            file("ERB/indices/BUFF.als", FileCategory::Als, "10,alias\n"),
        ],
    };
    let (mut session, progress, _manifest, initial_artifact) =
        title_session_fixture_with_manifest(manifest, 123_456);
    let drive_to_wait = |session: &mut RuntimeSession| {
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.phase == RuntimePhase::WaitingInput {
                break;
            }
        }
        assert_eq!(session.phase, RuntimePhase::WaitingInput);
    };
    drive_to_wait(&mut session);
    assert_eq!(
        projected_presentation_text(&session.presentation.snapshot()),
        "ALIAS=42"
    );
    drain(&mut session);

    session
        .reload_project(
            97,
            &ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: vec![FileChange::Upsert {
                    file: file("ERB/indices/BUFF.als", FileCategory::Als, "11,alias\n"),
                }],
            },
        )
        .unwrap();
    let target = session.artifact.as_ref().unwrap().artifact();
    let reloaded_artifact = target.manifest.artifact_id;
    assert_ne!(reloaded_artifact, initial_artifact);
    assert_eq!(
        target.project_data.static_data.deferred_indices.resolved["BUFF"].entries["alias"],
        11
    );

    // Calls made by the suspended title frame remain in its original program generation.
    let wait = session.operations.active_input().unwrap().wait.clone();
    session
        .handle_message(
            98,
            RuntimeMessage::Input(FrontendInput {
                wait_id: wait.wait_id,
                token: wait.submission_token,
                monotonic_time_ns: 0,
                intent: InputIntent::CommitText("1".into()),
                message_skip: false,
            }),
        )
        .unwrap();
    drive_to_wait(&mut session);
    assert_eq!(
        projected_presentation_text(&session.presentation.snapshot()),
        "ALIAS=42\n\nALIAS=42"
    );
    drain(&mut session);
    progress.lock().unwrap().clear();

    // Returning to title must use the committed reload, without reading or compiling sources.
    session.return_to_title(99).unwrap();
    let messages = drain(&mut session);
    assert!(
        messages
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
    answer_entropy(
        &mut session,
        return_to_title_entropy_request(messages),
        123_456,
    );
    drive_to_wait(&mut session);
    assert_eq!(
        projected_presentation_text(&session.presentation.snapshot()),
        "ALIAS=84"
    );
    assert_eq!(
        session.vm.as_ref().unwrap().vm().artifact_id(),
        reloaded_artifact
    );
    assert!(
        drain(&mut session)
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::ProjectLoadReport(_)))
    );
    let resumed_progress = progress.lock().unwrap();
    assert!(!resumed_progress.is_empty());
    assert!(
        resumed_progress
            .iter()
            .all(|value| { value.stage == ProjectProgressStage::InitializingMemory })
    );
    let replay_header = input_replay_records(&session).remove(0);
    assert_eq!(replay_header["origin"]["seed"], "123456");
    assert_eq!(replay_header["origin"]["trigger"], "return_to_title");
}

#[test]
fn return_to_title_retires_timeline_before_cancellation_and_entropy() {
    let (mut session, _progress, _manifest, _expected) = title_session_fixture();
    seed_title_timeline_retirement_state(&mut session);

    session.return_to_title(99).unwrap();
    assert_title_timeline_retired(&session);
    let messages = drain(&mut session);
    let clear_snapshot = messages
        .iter()
        .position(|message| {
            matches!(
                message,
                RuntimeMessage::PresentationSnapshot(snapshot)
                    if snapshot.history.logical_lines.is_empty()
                        && snapshot.resources.canvases.is_empty()
                        && snapshot.resources.sprites.is_empty()
            )
        })
        .expect("authoritative clear snapshot");
    assert!(messages[..clear_snapshot].iter().all(|message| !matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    let cancellation_position = messages
        .iter()
        .position(|message| {
            matches!(
                message,
                RuntimeMessage::CancelExternalRequest(request) if request.request_id == 77
            )
        })
        .expect("old external request cancellation");
    let entropy_position = messages
        .iter()
        .position(|message| {
            matches!(
                message,
                RuntimeMessage::ServiceRequest(request)
                    if request.operation == RANDOM_SEED_OPERATION
            )
        })
        .expect("return-to-title entropy request position");
    assert!(clear_snapshot < cancellation_position);
    assert!(cancellation_position < entropy_position);
}

#[test]
fn retained_title_program_is_released_on_failure_shutdown_load_and_reload() {
    let (mut session, _progress, manifest, _expected) = title_session_fixture();

    session.return_to_title(101).unwrap();
    assert!(session.vm.is_none());
    assert!(session.retained_title_program.is_some());
    let repeated = drain(&mut session);
    assert!(repeated.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(snapshot)
            if snapshot.history.logical_lines.is_empty()
                && snapshot.resources.canvases.is_empty()
    )));
    let repeated_entropy = repeated
        .into_iter()
        .find_map(|message| match message {
            RuntimeMessage::ServiceRequest(request)
                if request.operation == RANDOM_SEED_OPERATION =>
            {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("repeated return-to-title entropy request");
    session
        .handle_message(
            102,
            RuntimeMessage::ServiceResponse(ServiceResponse {
                request_id: repeated_entropy,
                result: ServiceResult::Error {
                    error: era_runtime_protocol::ServiceError {
                        code: "entropy.unavailable".into(),
                        message: "unavailable".into(),
                    },
                },
            }),
        )
        .unwrap();
    assert_eq!(session.phase, RuntimePhase::Faulted);
    assert!(session.vm.is_none());
    assert!(session.retained_title_program.is_some());
    session.shutdown(103).unwrap();
    assert!(session.retained_title_program.is_none());

    let (mut load_session, _progress, _old_manifest, _expected) = title_session_fixture();
    load_session.return_to_title(104).unwrap();
    assert!(load_session.retained_title_program.is_some());
    load_session.phase = RuntimePhase::Ready;
    load_session.operations = PendingOperations::default();
    let identity = crate::compiled_cache::project_identity(&manifest);
    load_session
        .load_project(
            105,
            ProjectLoadRequest {
                identity,
                manifest: Some(manifest),
                compiled_cache_transfer_id: None,
            },
        )
        .unwrap();
    assert!(load_session.retained_title_program.is_none());

    let (mut reload_session, _progress, _manifest, _expected) = title_session_fixture();
    reload_session.return_to_title(106).unwrap();
    assert!(reload_session.retained_title_program.is_some());
    reload_session.phase = RuntimePhase::Ready;
    reload_session.operations = PendingOperations::default();
    reload_session
        .reload_project(
            107,
            &ReloadProject {
                base_revision: 1,
                target_revision: 2,
                changes: Vec::new(),
            },
        )
        .unwrap();
    assert!(reload_session.retained_title_program.is_none());
}
