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

#[test]
#[allow(clippy::too_many_lines)]
fn one_input_normalization_is_scalar_default_and_activation_aware() {
    let submission = InteractionToken { epoch: 3, id: 1 };
    let long = InteractionToken { epoch: 3, id: 2 };
    let short = InteractionToken { epoch: 3, id: 3 };
    let mut pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 1,
            kind: WaitKind::StringValue,
            stability: WaitStability::StableInput,
            one_input: true,
            stop_message_skip: false,
            system_input: false,
            mouse_input: true,
            default_value: Some(era_runtime_protocol::ProtocolValue::String(
                "DEFAULT".into(),
            )),
            deadline_ns: None,
            display_time: false,
            timeout_message: None,
            submission_token: submission,
            countdown_remaining_ms: None,
            viewport_policy: era_runtime_protocol::InputViewportPolicy::FollowOutput,
        },
        result_name: Some("RESULTS".into()),
        choices: BTreeMap::from([
            (long, VmValue::String("LONG".into())),
            (short, VmValue::String("L".into())),
        ]),
        timeout_duration_ns: None,
        post_input: None,
    };
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("βx".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("β".into())))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("😀x".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("😀".into())))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText(String::new()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("DEFAULT".into())))
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), false,),
        Some(InputSubmission::Value(VmValue::String("L".into())))
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), true,),
        Some(InputSubmission::Value(VmValue::String("LONG".into())))
    );

    pending.wait.kind = WaitKind::IntegerValue;
    pending.wait.default_value = Some(era_runtime_protocol::ProtocolValue::Integer(42));
    pending.choices = BTreeMap::from([(long, VmValue::Integer(42)), (short, VmValue::Integer(4))]);
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("12".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(1)))
    );
    pending.wait.deadline_ns = Some(1_000);
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("34".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(3)))
    );
    pending.wait.kind = WaitKind::StringValue;
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("yz".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String("y".into())))
    );
    pending.wait.kind = WaitKind::IntegerValue;
    assert!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText("-1".into()),
            false,
        )
        .is_none()
    );
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), false,),
        Some(InputSubmission::Value(VmValue::Integer(4)))
    );

    pending.wait.kind = WaitKind::IntegerButton;
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), false,),
        Some(InputSubmission::Value(VmValue::Integer(4)))
    );
    pending.choices.remove(&short);
    assert!(input_value(&pending, submission, InputIntent::Activate(long), false,).is_none());
    assert_eq!(
        input_value(&pending, submission, InputIntent::Activate(long), true,),
        Some(InputSubmission::Value(VmValue::Integer(42)))
    );
}

#[test]
fn empty_string_input_without_a_default_remains_a_valid_string() {
    let submission = InteractionToken { epoch: 4, id: 1 };
    let empty_button = InteractionToken { epoch: 4, id: 2 };
    let mut pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 1,
            kind: WaitKind::StringValue,
            stability: WaitStability::StableInput,
            one_input: true,
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
        result_name: Some("RESULTS".into()),
        choices: BTreeMap::from([(empty_button, VmValue::String(String::new()))]),
        timeout_duration_ns: None,
        post_input: None,
    };

    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::CommitText(String::new()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String(String::new())))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::Activate(empty_button),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String(String::new())))
    );

    pending.wait.kind = WaitKind::StringButton;
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::Activate(empty_button),
            false,
        ),
        Some(InputSubmission::Value(VmValue::String(String::new())))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn one_input_activation_uses_the_loaded_allow_long_configuration() {
    fn run(allow_long: bool) -> (i64, String) {
        let mut session = RuntimeSession::new(RuntimeOptions::default());
        submit(
            &mut session,
            0,
            RuntimeMessage::ClientHello(ClientHello {
                runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
                client_name: "one-input-test".into(),
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
                files: vec![
                    SubmittedFile {
                        relative_path: "emuera.config".into(),
                        category: FileCategory::Configuration,
                        payload: FilePayload::Utf8(format!(
                            "Allow long input by mouse for ONEINPUT:{}\n",
                            if allow_long { "YES" } else { "NO" }
                        )),
                        content_hash: None,
                    },
                    SubmittedFile {
                        relative_path: "input.erb".into(),
                        category: FileCategory::Erb,
                        payload: FilePayload::Utf8(
                            "@SYSTEM_TITLE\nPRINTBUTTON \"42\", 42\nONEINPUT\nRESULT:42 = RESULT\nPRINTBUTTON \"LONG\", \"LONG\"\nONEINPUTS\nWAIT\nRETURN\n"
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
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let (wait_id, submission_token, activation) = {
            let pending = session.operations.active_input().unwrap();
            let activation = pending
                .choices
                .iter()
                .find_map(|(token, value)| (*value == VmValue::Integer(42)).then_some(*token))
                .unwrap();
            (
                pending.wait.wait_id,
                pending.wait.submission_token,
                activation,
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
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let (wait_id, submission_token, activation) = {
            let pending = session.operations.active_input().unwrap();
            let activation = pending
                .choices
                .iter()
                .find_map(|(token, value)| {
                    (*value == VmValue::String("LONG".into())).then_some(*token)
                })
                .unwrap();
            (
                pending.wait.wait_id,
                pending.wait.submission_token,
                activation,
            )
        };
        submit(
            &mut session,
            4,
            RuntimeMessage::Input(FrontendInput {
                wait_id,
                token: submission_token,
                monotonic_time_ns: 0,
                intent: InputIntent::Activate(activation),
                message_skip: false,
            }),
        );
        for _ in 0..16 {
            session.drive(RuntimeDriveBudget::default()).unwrap();
            if session.operations.active_input().is_some() {
                break;
            }
        }
        let vm = session.vm.as_ref().unwrap();
        (
            read_runtime_integer(vm, "RESULT", &[42], None).unwrap(),
            read_runtime_string(vm, "RESULTS").unwrap(),
        )
    }

    assert_eq!(run(false), (4, "L".into()));
    assert_eq!(run(true), (42, "LONG".into()));
}

#[test]
fn primitive_input_uses_runtime_selection_tokens_and_rejects_timeout_spoofing() {
    let submission = InteractionToken { epoch: 7, id: 1 };
    let selection = InteractionToken { epoch: 7, id: 2 };
    let pending = PendingInput {
        host_request: None,
        wait: InputWait {
            wait_id: 9,
            kind: WaitKind::PrimitiveMouseKey,
            stability: WaitStability::Transient,
            one_input: false,
            stop_message_skip: false,
            system_input: false,
            mouse_input: true,
            default_value: None,
            deadline_ns: Some(10),
            display_time: false,
            timeout_message: None,
            submission_token: submission,
            countdown_remaining_ms: None,
            viewport_policy: era_runtime_protocol::InputViewportPolicy::FollowOutput,
        },
        result_name: Some("RESULT".into()),
        choices: BTreeMap::from([(selection, VmValue::Integer(42))]),
        timeout_duration_ns: Some(10),
        post_input: None,
    };
    let input = era_runtime_protocol::PrimitiveInput {
        input_type: 1,
        result_1: 10,
        result_2: 20,
        result_3: 1,
        result_4: 3,
        selection_token: Some(selection),
    };
    assert_eq!(
        input_value(&pending, submission, InputIntent::Primitive(input), false),
        Some(InputSubmission::Primitive(PrimitiveResult {
            fields: [1, 10, 20, 1, 3],
            selection: Some(VmValue::Integer(42)),
        }))
    );
    assert_eq!(
        input_value(
            &pending,
            submission,
            InputIntent::Activate(selection),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(42)))
    );
    assert!(
        input_value(
            &pending,
            InteractionToken { epoch: 7, id: 99 },
            InputIntent::Activate(selection),
            false,
        )
        .is_none()
    );
    assert!(
        input_value(
            &pending,
            submission,
            InputIntent::Primitive(era_runtime_protocol::PrimitiveInput {
                input_type: 4,
                result_1: 0,
                result_2: 0,
                result_3: 0,
                result_4: 0,
                selection_token: None,
            }),
            false,
        )
        .is_none()
    );
}

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
