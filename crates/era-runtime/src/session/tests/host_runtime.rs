use super::*;

#[test]
fn goto_into_case_body_emits_a_nonfatal_warning_and_continues() {
    let mut session = negotiated_session();
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nGOTO CHOICE\nSELECTCASE 0\nCASE 0\n$CHOICE\nPRINTL reached\nENDSELECT\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert!(drain(&mut session).iter().any(|message| {
        matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
    }));
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
    let messages = drain(&mut session);

    let diagnostic = messages.iter().find_map(|message| match message {
        RuntimeMessage::Diagnostic(diagnostic)
            if diagnostic.code == "vm.control_flow.goto_into_structured_block" =>
        {
            Some(diagnostic)
        }
        _ => None,
    });
    let diagnostic = diagnostic.expect("cross-block GOTO warning");
    assert_eq!(diagnostic.level, RuntimeLogLevel::Warning);
    assert!(diagnostic.message.contains("avoid jumping"));
    assert_eq!(
        diagnostic
            .source
            .as_ref()
            .map(|source| source.relative_path.as_str()),
        Some("main.erb")
    );
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::Fault(_))),
        "{messages:#?}"
    );
    assert!(projected_presentation_text(&session.presentation.snapshot()).contains("reached"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn gcreatefromfile_defaults_to_content_directory_and_replays_dynamic_sprite() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client = capabilities();
    client.graphics = true;
    client.html = true;
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
            client_name: "content-directory-graphics-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
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
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nRESULT = GCREATEFROMFILE(1, \"dummy.webp\")\nRESULT:1 = SPRITECREATE(\"FACE_1\", 1)\nHTML_PRINT \"<img src='FACE_1'>\"\nWAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "resources/dummy.webp".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let metadata_request = drain(&mut session)
        .into_iter()
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
            request_id: metadata_request,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&ImageMetadataResponse {
                        width: 300,
                        height: 300,
                        format: "webp".into(),
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
    submit(
        &mut session,
        3,
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

    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(),
        1
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[1], None).unwrap(),
        1
    );
    let replay = session.presentation.snapshot().resources;
    let face = replay
        .sprites
        .iter()
        .find(|sprite| sprite.name == "FACE_1")
        .expect("dynamic face sprite");
    assert_eq!(face.canvas_id, Some(1));
    let canvas = replay
        .canvases
        .iter()
        .find(|canvas| canvas.canvas_id == 1)
        .expect("content image canvas");
    let CanvasReplayCommand::DrawSprite { name, .. } = &canvas.commands[0] else {
        panic!("content image canvas must reference a frontend-owned resource");
    };
    assert!(replay.sprites.iter().any(|sprite| {
        sprite.name == *name && sprite.frames[0].resource_id == "resources/dummy.webp"
    }));
}

#[test]
fn retired_drawing_backend_queries_keep_the_reference_compatibility_value() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "drawing-query-test".into(),
            features: Vec::new(),
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
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTFORML %GETCONFIGS(\"TextDrawingMode\")%|%GETCONFIGS(\"Drawing interface\")%|%GETCONFIGS(\"描画インターフェース\")%|%GETCONFIGS(\"  textdrawingmode  \")%\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert!(
        projected_presentation_text(&session.presentation.snapshot())
            .contains("TEXTRENDERER|TEXTRENDERER|TEXTRENDERER|TEXTRENDERER")
    );
}

#[test]
fn audio_commands_project_canonical_sound_directory_resources() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut client_capabilities = capabilities();
    client_capabilities.audio = true;
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "audio-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client_capabilities,
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
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nPLAYBGM \"theme.mp3\"\nPLAYSOUND \"door.mp3\"\nSETSOUNDVOLUME 25\nPLAYSOUND \"knock.mp3\"\nSTOPSOUND\nWAIT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "sound/theme.mp3".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![1, 2, 3])),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "sound/door.mp3".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![4, 5, 6])),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "sound/knock.mp3".into(),
                    category: FileCategory::Resource,
                    payload: FilePayload::Bytes(ProtocolBytes::new(vec![7, 8, 9])),
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

    let mut messages = Vec::new();
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        let audio_effects = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::EffectBatch(batch) => Some(batch.effects.len()),
                _ => None,
            })
            .sum::<usize>();
        if audio_effects >= 5 {
            break;
        }
    }

    assert_audio_effect(
        &messages,
        1,
        AudioEffectAction::Play,
        Some("sound/theme.mp3"),
    );
    for resource in ["sound/door.mp3", "sound/knock.mp3"] {
        assert_audio_effect(&messages, 0, AudioEffectAction::Play, Some(resource));
    }
    for action in [AudioEffectAction::SetVolume, AudioEffectAction::Stop] {
        assert_audio_effect(&messages, 0, action, None);
    }
    let audio = session.presentation.snapshot().audio;
    assert_eq!(audio.len(), 1);
    assert_eq!(audio[0].channel_id, 1);
    assert_eq!(audio[0].resource_id, "sound/theme.mp3");
    assert_eq!(audio[0].volume_millionths, 1_000_000);
    assert!(audio[0].playing);
}

fn assert_audio_effect(
    messages: &[RuntimeMessage],
    channel_id: u64,
    action: AudioEffectAction,
    resource_id: Option<&str>,
) {
    assert!(
        messages.iter().any(|message| matches!(
            message,
            RuntimeMessage::EffectBatch(batch)
                if batch.effects.iter().any(|effect| matches!(
                    &effect.kind,
                    EffectKind::Audio(audio)
                        if audio.channel_id == channel_id
                            && audio.action == action
                            && resource_id.is_none_or(|expected| {
                                audio.resource_id.as_deref() == Some(expected)
                            })
                ))
        )),
        "missing channel {channel_id} {action:?} audio effect for {resource_id:?}: {messages:#?}"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_message_skip_input_drains_non_value_waits_until_forcewait() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "message-skip-test".into(),
            features: Vec::new(),
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
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "message-skip.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTL first\nWAIT\nPRINTL second\nWAITANYKEY\nPRINTL third\nTWAIT 100, 1\nPRINTL fourth\nFORCEWAIT\nPRINTL after\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    drain(&mut session);
    let (initial_wait_id, initial_token) = {
        let pending = session.operations.active_input().unwrap();
        (pending.wait.wait_id, pending.wait.submission_token)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: initial_wait_id,
            token: initial_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Enter,
            message_skip: true,
        }),
    );

    let mut messages = Vec::new();
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session
            .operations
            .active_input()
            .is_some_and(|input| input.wait.stop_message_skip)
        {
            break;
        }
    }

    let pending = session.operations.active_input().expect("force wait");
    assert!(pending.wait.stop_message_skip);
    assert!(!session.message_skip);
    let output = session.presentation.log_text(false);
    assert!(output.contains("fourth"));
    assert!(!output.contains("after"));
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, RuntimeMessage::CommandRejected(_)))
    );
    assert!(!messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) if !wait.stop_message_skip
    )));
    assert_eq!(
        messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::WaitChanged(WaitChange::Closed(wait_id)) => Some(*wait_id),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![initial_wait_id]
    );
}

#[test]
fn message_skip_stops_before_a_value_wait() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "message-skip-value-test".into(),
            features: Vec::new(),
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
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "message-skip-value.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nWAIT\nINPUT\nPRINTL after\nRETURN\n".into(),
                ),
                content_hash: None,
            }],
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    drain(&mut session);
    let (wait_id, token) = {
        let pending = session.operations.active_input().unwrap();
        (pending.wait.wait_id, pending.wait.submission_token)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 0,
            intent: InputIntent::Enter,
            message_skip: true,
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session
            .operations
            .active_input()
            .is_some_and(|input| input.wait.kind == WaitKind::IntegerValue)
        {
            break;
        }
    }
    let pending = session.operations.active_input().expect("value wait");
    assert_eq!(pending.wait.kind, WaitKind::IntegerValue);
    assert!(!session.message_skip);
    assert!(!session.presentation.log_text(false).contains("after"));
}

#[test]
fn project_load_start_and_print_cross_the_message_boundary() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);

    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nSKIPDISP 1\nSKIPDISP 0\nPRINTFORML TITLE_CHARANUM={CHARANUM}\nPRINTL ORACLE_READY\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "CHARA0.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("番号,0\n名前,initial\n".into()),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(loaded.iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectLoadReport(report) if report.success
    )));
    assert_eq!(session.phase(), RuntimePhase::Ready);

    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let initial = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 2,
        })
        .expect("start");
    assert_eq!(initial.runtime_transitions, 2);
    let mut output = drain(&mut session);
    let yielded = session
        .drive(RuntimeDriveBudget {
            maximum_vm_instructions: 100_000,
            maximum_runtime_transitions: 1,
        })
        .expect("bounded ready host call");
    assert_eq!(yielded.state, RuntimeDriveState::MoreWork);
    let report = session.drive(RuntimeDriveBudget::default()).expect("run");
    assert!(
        report.runtime_transitions >= 3,
        "ready host calls should be batched in one bounded runtime drive: {report:?}"
    );
    assert_eq!(session.random_seed(), Some(1));
    output.extend(drain(&mut session));
    assert!(output.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    let snapshot = session.presentation.snapshot();
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("ORACLE_READY"))
    );
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("TITLE_CHARANUM=0"))
    );
}

#[test]
fn linecount_drives_clearline_and_bounded_padding_loops() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "linecount-test".into(),
            features: Vec::new(),
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
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nCALL ORACLE_LINECOUNT\nWAIT\nRETURN\n".into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "linecount.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        include_str!(
                            "../../../../../tools/runtime-tester/fixture-reference/erb/linecount.erb"
                        )
                        .into(),
                    ),
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
    for _ in 0..20 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        drain(&mut session);
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(session.phase(), RuntimePhase::WaitingInput);
    assert_eq!(session.presentation.logical_line_count(), 3);
    let vm = session.vm.as_ref().unwrap();
    assert_eq!(read_runtime_integer(vm, "RESULT", &[50], None).unwrap(), 2);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[51], None).unwrap(), 1);
    assert_eq!(read_runtime_integer(vm, "RESULT", &[52], None).unwrap(), 3);
    let snapshot = session.presentation.snapshot();
    assert_eq!(snapshot.history.logical_lines.len(), 3);
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line) == "one")
    );
}

include!("host_runtime_continued.rs");
