use super::*;

#[test]
fn phase_changes_emit_state_without_redundant_logs() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());

    session.set_phase(RuntimePhase::Ready).unwrap();

    let messages = drain(&mut session);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::StateChanged(state) if state.phase == RuntimePhase::Ready
    )));
    assert!(
        messages
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::Log(_)))
    );
}

#[test]
fn projection_observation_updates_draw_line_string_width() {
    let build = build_project(
        &ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8("@SYSTEM_TITLE\nRETURN\n".into()),
                content_hash: None,
            }],
        },
        None,
    );
    let artifact = build.artifact.expect("valid project");
    let draw_line = artifact
        .artifact()
        .globals
        .iter()
        .find(|global| global.name == "DRAWLINESTR")
        .expect("DRAWLINESTR")
        .key;
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.vm = Some(RuntimeVm::new(artifact, VmConfig::default()));

    session
        .observe_projection(
            1,
            ProjectionObservation {
                environment_revision: 1,
                presentation_revision: session.presentation.revision(),
                client_size: ProjectionSize {
                    width: ProjectionLength(1_395),
                    height: ProjectionLength(768),
                },
                projection_space_revision: 1,
                line_columns: 198,
                text_box: String::new(),
                transform: ProjectionTransform {
                    x_numerator: 1,
                    x_denominator: 1,
                    y_numerator: 1,
                    y_denominator: 1,
                    origin_x: ProjectionLength(0),
                    origin_y: ProjectionLength(0),
                },
            },
        )
        .unwrap();

    assert_eq!(session.line_columns, 198);
    assert_eq!(
        session
            .vm
            .as_ref()
            .unwrap()
            .vm()
            .read_variable(draw_line, &[], None),
        Ok(VmValue::String("-".repeat(198)))
    );
}

#[test]
fn presentation_updates_are_coalesced_until_the_drive_boundary() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());

    for fragment in ["first", "second", "third"] {
        session
            .presentation
            .append_print_text(fragment.into(), false, false);
        session.emit_presentation().unwrap();
    }

    assert!(
        session.outbound.is_empty(),
        "intermediate current-line projections must not be serialized"
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let messages = drain(&mut session);
    let updates = messages
        .iter()
        .filter(|message| {
            matches!(
                message,
                RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
            )
        })
        .count();

    assert_eq!(updates, 1);
    let snapshot = session.presentation.snapshot();
    let text = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| &line.runs)
        .filter_map(|run| match run {
            DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "firstsecondthird");
}

#[test]
fn resource_replay_is_materialized_once_when_a_deferred_frame_is_published() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.presentation.snapshot_for_delivery();

    assert!(!session.sync_resource_replay());
    assert!(!session.sync_resource_replay());
    session.emit_presentation().unwrap();
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let deferred = drain(&mut session);
    assert!(session.presentation.resource_replay_stale());
    assert!(deferred.iter().all(|message| {
        match message {
            RuntimeMessage::PresentationDelta(delta) => !delta
                .operations
                .iter()
                .any(|operation| matches!(operation, PresentationOperation::SetResources { .. })),
            _ => true,
        }
    }));

    session.presentation.set_redraw(false);
    session.presentation.set_redraw(true);
    session.emit_presentation().unwrap();
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let published = drain(&mut session);
    let resource_updates = published
        .iter()
        .filter_map(|message| match message {
            RuntimeMessage::PresentationDelta(delta) => Some(&delta.operations),
            _ => None,
        })
        .flatten()
        .filter(|operation| matches!(operation, PresentationOperation::SetResources { .. }))
        .count();

    assert_eq!(resource_updates, 1);
    assert!(!session.presentation.resource_replay_stale());
}

#[test]
fn handshake_selects_only_implemented_features() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::Audio, RuntimeFeature::TimedInput],
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("drive");
    let messages = drain(&mut session);
    let RuntimeMessage::ServerHello(hello) = &messages[0] else {
        panic!("expected server hello");
    };
    assert_eq!(hello.selected_version, RUNTIME_PROTOCOL_VERSION);
    assert!(hello.features.contains(&RuntimeFeature::TimedInput));
    assert!(!hello.features.contains(&RuntimeFeature::Audio));
    assert_eq!(hello.selected_capabilities.storage, capabilities().storage);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::Log(RuntimeLog {
            level: RuntimeLogLevel::Debug,
            message,
        }) if message.contains("handshake complete")
    )));
}

#[test]
fn key_macro_edits_emit_canonical_state_and_persist_through_frontend_storage() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Ready;
    session.epoch = SessionEpoch(1);
    session.negotiated_features.insert(RuntimeFeature::Storage);
    session
        .negotiated_features
        .insert(RuntimeFeature::KeyMacros);
    session.storage_capabilities = capabilities().storage;
    session
        .apply_key_macro_command(
            7,
            KeyMacroCommand::Store {
                group: 1,
                slot: 2,
                text: "abc".into(),
            },
        )
        .unwrap();
    let messages = drain(&mut session);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::KeyMacroStateChanged(state)
            if state.entries[era_runtime_protocol::KEY_MACRO_SLOTS + 2] == "abc"
                && state.serialized.contains("G1:マクロキーF3:abc")
    )));
    let request_id = messages
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::StorageRequest(request) if request.relative_path == "macro.txt" => {
                Some(request.request_id)
            }
            _ => None,
        })
        .expect("macro persistence request");
    assert_eq!(session.phase, RuntimePhase::WaitingExternal);
    session
        .complete_storage(
            8,
            StorageResponse {
                request_id,
                result: StorageResult::Written {
                    revision: Some("1".into()),
                },
            },
        )
        .unwrap();
    assert_eq!(session.phase, RuntimePhase::Ready);
}

#[test]
fn key_macro_activation_recalls_runtime_text_without_completing_the_wait() {
    let token = InteractionToken { epoch: 1, id: 4 };
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::WaitingInput;
    session.epoch = SessionEpoch(1);
    session
        .negotiated_features
        .insert(RuntimeFeature::KeyMacros);
    assert!(session.key_macros.store(2, 3, "(ab)*2\\nnext".into()));
    session.operations.activate_input(PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 9,
            kind: WaitKind::StringValue,
            stability: WaitStability::StableInput,
            one_input: false,
            stop_message_skip: false,
            system_input: true,
            mouse_input: false,
            default_value: None,
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token: token,
            countdown_remaining_ms: None,
        },
        result_name: Some("RESULTS".into()),
        choices: BTreeMap::new(),
        timeout_duration_ns: None,
        post_input: None,
    });
    session
        .complete_input(
            7,
            FrontendInput {
                wait_id: 9,
                token,
                monotonic_time_ns: 0,
                intent: InputIntent::ActivateKeyMacro { group: 2, slot: 3 },
                message_skip: false,
            },
        )
        .unwrap();
    assert_eq!(session.text_box, "(ab)*2\\nnext");
    assert!(session.operations.active_input().is_some());
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectionState(state) if state.text_box == "(ab)*2\\nnext"
    )));
}

#[test]
fn project_analysis_is_one_shot_and_does_not_replace_loaded_state() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.state = SessionState::Active;
    session.phase = RuntimePhase::Negotiating;
    session.epoch = SessionEpoch(1);
    session
        .negotiated_features
        .insert(RuntimeFeature::ProjectAnalysis);
    session
        .analyze_project(
            3,
            &era_runtime_protocol::ProjectAnalysisRequest {
                manifest: ProjectManifest {
                    project_revision: 4,
                    files: vec![SubmittedFile {
                        relative_path: "unused.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8("@UNUSED\nRETURN\n".into()),
                        content_hash: None,
                    }],
                },
                selected_erb_paths: Vec::new(),
                debug_mode: false,
            },
        )
        .unwrap();
    assert!(session.project_snapshot.is_none());
    assert_eq!(session.phase, RuntimePhase::Negotiating);
    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectAnalysisReport(report) if report.success
    )));
}

include!("protocol_handshake_continued.rs");
