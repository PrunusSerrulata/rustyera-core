use super::*;

#[test]
fn identical_phase_keeps_the_presentation_barrier_without_a_state_revision() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.presentation.append_text("pending".into(), false);
    session.emit_presentation().unwrap();
    let revision = session.revision;
    let phase = session.phase;

    session.set_phase(phase).unwrap();
    let messages = drain(&mut session);

    assert_eq!(session.revision, revision);
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    assert!(
        messages
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::StateChanged(_)))
    );
}

#[test]
fn identical_projection_keeps_the_presentation_barrier_without_republication() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.emit_projection_state().unwrap();
    drain(&mut session);
    session.presentation.append_text("pending".into(), false);
    session.emit_presentation().unwrap();

    session.emit_projection_state().unwrap();
    let messages = drain(&mut session);

    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    assert!(
        messages
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::ProjectionState(_)))
    );
}

#[test]
fn frontend_projection_observation_invalidates_runtime_delivery_deduplication() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.text_box = "runtime-value".into();
    session.emit_projection_state().unwrap();
    drain(&mut session);

    session
        .observe_projection(
            1,
            ProjectionObservation {
                environment_revision: 1,
                presentation_revision: session.presentation.revision(),
                client_size: ProjectionSize {
                    width: ProjectionLength(760),
                    height: ProjectionLength(480),
                },
                projection_space_revision: 1,
                line_columns: DEFAULT_LINE_COLUMNS,
                text_box: "frontend-value".into(),
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
    session.text_box = "runtime-value".into();
    session.emit_projection_state().unwrap();

    assert!(drain(&mut session).iter().any(|message| matches!(
        message,
        RuntimeMessage::ProjectionState(state) if state.text_box == "runtime-value"
    )));
}

#[test]
fn hello_invalidates_projection_delivery_deduplication() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.emit_projection_state().unwrap();
    drain(&mut session);
    session
        .hello(
            0,
            &ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "projection-reset-test".into(),
                features: Vec::new(),
                requested_limits: RuntimeOptions::default().limits,
                capabilities: capabilities(),
                preferred_locales: vec!["en".into()],
                configuration_profile: None,
            },
        )
        .unwrap();
    drain(&mut session);

    session.emit_projection_state().unwrap();
    assert_eq!(
        drain(&mut session)
            .iter()
            .filter(|message| matches!(message, RuntimeMessage::ProjectionState(_)))
            .count(),
        1
    );
}

#[test]
fn flow_input_configuration_shapes_system_waits() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let ordinary = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    assert_eq!(ordinary.kind, WaitKind::IntegerButton);
    session.flow_input_enabled = true;
    session.flow_input_default = 7;
    let integer = session.system_wait(InteractionToken { epoch: 0, id: 2 });
    assert_eq!(integer.kind, WaitKind::IntegerValue);
    assert_eq!(
        integer.default_value,
        Some(era_runtime_protocol::ProtocolValue::Integer(7))
    );
    assert!(integer.mouse_input);
    session.flow_input_string = true;
    session.flow_input_default_string = "fallback".into();
    let string = session.system_wait(InteractionToken { epoch: 0, id: 3 });
    assert_eq!(string.kind, WaitKind::StringValue);
    assert_eq!(
        string.default_value,
        Some(era_runtime_protocol::ProtocolValue::String(
            "fallback".into()
        ))
    );
}

#[test]
fn message_skip_only_completes_non_value_waits_without_a_stop_barrier() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    let mut wait = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    wait.system_input = false;
    for kind in [WaitKind::EnterKey, WaitKind::AnyKey, WaitKind::Void] {
        wait.kind = kind;
        wait.stop_message_skip = false;
        assert!(message_skip_submission(&wait).is_some(), "{kind:?}");
    }
    for kind in [
        WaitKind::IntegerValue,
        WaitKind::StringValue,
        WaitKind::AnyValue,
        WaitKind::IntegerButton,
        WaitKind::StringButton,
        WaitKind::PrimitiveMouseKey,
    ] {
        wait.kind = kind;
        wait.stop_message_skip = false;
        assert!(message_skip_submission(&wait).is_none(), "{kind:?}");
    }
    wait.kind = WaitKind::EnterKey;
    wait.stop_message_skip = true;
    assert!(message_skip_submission(&wait).is_none());
}

#[test]
fn visible_button_activation_completes_enter_and_any_key_waits() {
    let submission = InteractionToken { epoch: 2, id: 1 };
    let button = InteractionToken { epoch: 2, id: 2 };
    let mut pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 1,
            kind: WaitKind::AnyKey,
            stability: WaitStability::StableInput,
            one_input: false,
            stop_message_skip: false,
            system_input: false,
            mouse_input: true,
            default_value: None,
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token: submission,
            countdown_remaining_ms: None,
            viewport_policy: era_runtime_protocol::InputViewportPolicy::FollowOutput,
        },
        result_name: Some("RESULT".into()),
        choices: BTreeMap::from([(button, VmValue::String("0".into()))]),
        timeout_duration_ns: None,
        post_input: None,
    };

    for kind in [WaitKind::AnyKey, WaitKind::EnterKey] {
        pending.wait.kind = kind;
        assert_eq!(
            input_value(&pending, submission, InputIntent::Activate(button), false),
            Some(InputSubmission::Value(VmValue::Integer(0))),
            "{kind:?}"
        );
    }
    assert!(
        input_value(
            &pending,
            InteractionToken { epoch: 2, id: 99 },
            InputIntent::Activate(button),
            false,
        )
        .is_none()
    );
    assert!(
        input_value(
            &pending,
            submission,
            InputIntent::Activate(InteractionToken { epoch: 2, id: 99 }),
            false,
        )
        .is_none()
    );
}

#[test]
fn visible_button_activation_closes_a_real_any_key_wait() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "any-key-button-test".into(),
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "any-key-button.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTBUTTON \"help body\", 0\nPRINTL\nWAITANYKEY\nPRINTL closed\nWAIT\nRETURN\n"
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
    let (wait_id, submission_token, activation) = {
        let pending = session.operations.active_input().expect("AnyKey wait");
        assert_eq!(pending.wait.kind, WaitKind::AnyKey);
        (
            pending.wait.wait_id,
            pending.wait.submission_token,
            *pending.choices.keys().next().expect("visible button token"),
        )
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(activation),
            message_skip: false,
        }),
    );

    let mut messages = Vec::new();
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        messages.extend(drain(&mut session));
        if session
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.wait_id != wait_id)
        {
            break;
        }
    }

    let pending = session
        .operations
        .active_input()
        .expect("following Enter wait");
    assert_ne!(pending.wait.wait_id, wait_id);
    assert_eq!(pending.wait.kind, WaitKind::EnterKey);
    assert!(session.presentation.log_text(false).contains("closed"));
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::WaitChanged(WaitChange::Closed(closed)) if *closed == wait_id
    )));
    assert!(
        messages
            .iter()
            .all(|message| !matches!(message, RuntimeMessage::CommandRejected(_)))
    );
}

#[test]
fn frontend_monotonic_time_rebases_onto_restored_logical_time() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.logical_time_ns = 100;
    assert_eq!(session.observe_frontend_time(5), 100);
    assert_eq!(session.observe_frontend_time(15), 110);
    assert_eq!(session.observe_frontend_time(9), 110);
}
