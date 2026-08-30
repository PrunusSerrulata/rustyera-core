use super::*;

#[test]
fn secondary_mouse_down_sets_message_skip_before_the_interpreter_resumes() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nWAIT\nIF MESSKIP\nSEEN = 1\nENDIF\nFORCEWAIT\nRETURN\n",
    );
    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let token = pending.wait.submission_token;

    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: false,
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::DeviceStateChanged(era_runtime_protocol::DeviceStateChanged {
            event_sequence: 1,
            toggle: false,
            repeat: false,
            device: era_runtime_protocol::InputDeviceKind::Mouse,
            code: 2,
            pressed: true,
            x: 0,
            y: 0,
            monotonic_time_ns: 1,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();

    assert_eq!(runtime_integer(&session, "SEEN"), 1);
    assert!(
        session
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.stop_message_skip)
    );
    assert!(!session.message_skip);
}

#[test]
fn running_message_skip_defers_presentation_until_the_next_input_boundary() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM OBSERVED\nWAIT\nFOR LOCAL, 0, 4\nCLEARLINE 1\nPRINTFORML frame {LOCAL}\nTWAIT 8, 0\nIF MESSKIP()\nOBSERVED += 1\nENDIF\nNEXT\nINPUT\nRETURN\n",
    );
    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let token = pending.wait.submission_token;
    drain(&mut session);
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: true,
        }),
    );

    let mut boundary_messages = Vec::new();
    for _ in 0..256 {
        session
            .drive(RuntimeDriveBudget {
                maximum_vm_instructions: 4,
                maximum_runtime_transitions: 1,
            })
            .unwrap();
        let messages = drain(&mut session);
        if session.message_skip {
            assert!(
                !messages.iter().any(|message| matches!(
                    message,
                    RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
                )),
                "running skip projected an intermediate frame: {messages:#?}"
            );
        } else {
            boundary_messages = messages;
            break;
        }
    }

    assert_wait(&session, WaitKind::IntegerValue);
    assert!(boundary_messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    assert_eq!(runtime_integer(&session, "OBSERVED"), 4);
    let output = session.presentation.log_text(false);
    assert!(output.ends_with("frame 3\r\n"), "{output:?}");
    for discarded in ["frame 0", "frame 1", "frame 2"] {
        assert!(!output.contains(discarded), "{output:?}");
    }
}

#[test]
fn running_redraw_disabled_defers_presentation_until_the_next_input_boundary() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\nWAIT\nREDRAW 0\nFOR LOCAL, 0, 4\nCLEARLINE 1\nPRINTFORML frame {LOCAL}\nNEXT\nINPUT\nRETURN\n",
    );
    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let token = pending.wait.submission_token;
    drain(&mut session);
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: false,
        }),
    );
    drain(&mut session);

    let mut boundary_messages = Vec::new();
    for _ in 0..256 {
        let redraw_was_already_disabled =
            !session.presentation.redraw_enabled() && session.operations.active_input().is_none();
        session
            .drive(RuntimeDriveBudget {
                maximum_vm_instructions: 4,
                maximum_runtime_transitions: 1,
            })
            .unwrap();
        let messages = drain(&mut session);
        if session.operations.active_input().is_some() {
            boundary_messages.extend(messages);
            break;
        }
        if redraw_was_already_disabled {
            assert!(
                !messages.iter().any(|message| matches!(
                    message,
                    RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
                )),
                "redraw-disabled execution projected an intermediate frame: {messages:#?}"
            );
        }
    }

    assert_wait(&session, WaitKind::IntegerValue);
    assert!(boundary_messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
    let output = session.presentation.log_text(false);
    assert!(output.ends_with("frame 3\r\n"), "{output:?}");
    for discarded in ["frame 0", "frame 1", "frame 2"] {
        assert!(!output.contains(discarded), "{output:?}");
    }
}

#[test]
fn present_now_follows_the_presentation_revision_it_observes() {
    let mut session =
        start_input_project("@SYSTEM_TITLE\nPRINTL visible\nREDRAW 2\nWAIT\nRETURN\n");
    let messages = drain(&mut session);
    let (effect_index, effect_revision) = messages
        .iter()
        .enumerate()
        .find_map(|(index, message)| match message {
            RuntimeMessage::EffectBatch(batch) => batch.effects.iter().find_map(|effect| {
                if let EffectKind::PresentNow {
                    presentation_revision,
                } = &effect.kind
                {
                    Some((index, *presentation_revision))
                } else {
                    None
                }
            }),
            _ => None,
        })
        .expect("REDRAW 2 present-now effect");
    let (presentation_index, presentation_revision) = messages[..effect_index]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| match message {
            RuntimeMessage::PresentationSnapshot(snapshot) => Some((index, snapshot.revision)),
            RuntimeMessage::PresentationDelta(delta) => Some((index, delta.new_revision)),
            _ => None,
        })
        .expect("presentation update before present-now effect");

    assert!(presentation_index < effect_index);
    assert_eq!(presentation_revision, effect_revision);
    assert!(!session.presentation.redraw_enabled());
}

#[test]
fn negative_display_line_query_does_not_publish_a_skipped_frame() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\nWAIT\nPRINTL pending\nRESULTS '= GETDISPLAYLINE(-1)\nRESULT = GETKEY(65)\nRETURN\n",
    );
    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let token = pending.wait.submission_token;
    drain(&mut session);
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: true,
        }),
    );

    let mut messages = Vec::new();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        let batch = drain(&mut session);
        let reached_get_key = batch.iter().any(|message| {
            matches!(
                message,
                RuntimeMessage::ServiceRequest(request)
                    if request.kind == ServiceKind::InputState
                        && request.operation == GET_KEY_STATE_OPERATION
            )
        });
        messages.extend(batch);
        if reached_get_key {
            break;
        }
    }

    assert!(session.message_skip);
    assert_eq!(runtime_string(&session, "RESULTS"), "");
    assert!(messages.iter().any(|message| matches!(
        message,
        RuntimeMessage::ServiceRequest(request)
            if request.kind == ServiceKind::InputState
                && request.operation == GET_KEY_STATE_OPERATION
    )));
    assert!(messages.iter().all(|message| !matches!(
        message,
        RuntimeMessage::PresentationSnapshot(_) | RuntimeMessage::PresentationDelta(_)
    )));
}

#[test]
fn repeated_input_set_executes_every_segment_across_enter_waits() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM LEARNED\nFOR LOCAL, 0, 2\nINPUT\nIF RESULT == 412\nLEARNED += 1\nENDIF\nWAIT\nNEXT\nINPUT\nRETURN\n",
    );

    submit_text(&mut session, 3, "(412\\n\\e\\n)*2");
    drive_input_set(&mut session);

    assert_eq!(runtime_integer(&session, "LEARNED"), 2);
    assert!(session.queued_input.is_empty());
    assert!(!session.message_skip);
    assert_eq!(
        input_replay_records(&session)
            .into_iter()
            .skip(1)
            .map(|record| record["action"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        ["text", "enter", "text", "enter"]
    );
}

#[test]
fn invalid_segment_does_not_discard_the_following_valid_segment() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM ACCEPTED\nINPUT\nIF RESULT == 412\nACCEPTED = 1\nENDIF\nWAIT\nRETURN\n",
    );

    submit_text(&mut session, 3, "abc\\n412");
    drive_input_set(&mut session);

    assert_eq!(runtime_integer(&session, "ACCEPTED"), 1);
    assert!(session.queued_input.is_empty());
    let records = input_replay_records(&session);
    assert_eq!(records[0]["step_count"], 1);
    assert_eq!(records[1]["result"]["value"], "412");
}

#[test]
fn system_command_syntax_is_only_special_in_the_original_submission() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIMS SEEN\nINPUTS\nINPUTS\nSEEN '= %RESULTS%\nWAIT\nRETURN\n",
    );

    submit_text(&mut session, 3, "first\\n@CONFIG");
    drive_input_set(&mut session);

    assert_eq!(runtime_string(&session, "SEEN"), "@CONFIG");
    assert!(session.queued_input.is_empty());
}

#[test]
fn each_drive_consumes_at_most_one_queued_segment() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM FIRST\n#DIM SECOND\n#DIM THIRD\nINPUT\nFIRST = RESULT\nINPUT\nSECOND = RESULT\nINPUT\nTHIRD = RESULT\nWAIT\nRETURN\n",
    );
    submit_text(&mut session, 3, "1\\n2\\n3");

    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "FIRST"), 1);
    assert_eq!(runtime_integer(&session, "SECOND"), 0);
    assert_eq!(session.queued_input.len(), 2);

    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "SECOND"), 2);
    assert_eq!(runtime_integer(&session, "THIRD"), 0);
    assert_eq!(session.queued_input.len(), 1);

    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "THIRD"), 3);
    assert!(session.queued_input.is_empty());
}

#[test]
fn void_timed_wait_is_bypassed_without_consuming_its_segment() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nTWAIT 1000, 1\nINPUT\nSEEN = RESULT\nWAIT\nRETURN\n",
    );
    submit_text(&mut session, 3, "1\\n2\\e");
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_wait(&session, WaitKind::Void);
    assert_eq!(queued_text(&session), Some("2"));

    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_wait(&session, WaitKind::IntegerValue);
    assert_eq!(queued_text(&session), Some("2"));
    assert_eq!(runtime_integer(&session, "ISTIMEOUT"), 0);

    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "SEEN"), 2);
    assert!(session.queued_input.is_empty());
}

#[test]
fn ordinary_twait_and_forcewait_consume_explicit_segments() {
    for wait in ["TWAIT 1000, 0", "FORCEWAIT"] {
        let source = format!(
            "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\n{wait}\nINPUT\nSEEN = RESULT\nWAIT\nRETURN\n"
        );
        let mut session = start_input_project(&source);
        submit_text(&mut session, 3, "1\\n\\e\\n7");
        drive_input_set(&mut session);
        assert_eq!(runtime_integer(&session, "SEEN"), 7, "{wait}");
        assert!(session.queued_input.is_empty(), "{wait}");
        assert!(!session.message_skip, "{wait}");
    }
}

#[test]
fn message_skip_stops_at_value_and_forcewait_barriers() {
    for barrier in ["INPUT", "FORCEWAIT\nINPUT"] {
        let source = format!(
            "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nWAIT\n{barrier}\nSEEN = RESULT\nWAIT\nRETURN\n"
        );
        let mut session = start_input_project(&source);
        submit_text(&mut session, 3, "1\\n\\e\\n7\\n8");
        session.drive(RuntimeDriveBudget::default()).unwrap();
        session.drive(RuntimeDriveBudget::default()).unwrap();
        assert!(!session.message_skip, "{barrier}");
        assert_eq!(queued_text(&session), Some("7"), "{barrier}");
        assert_wait(
            &session,
            if barrier == "INPUT" {
                WaitKind::IntegerValue
            } else {
                WaitKind::EnterKey
            },
        );
    }
}

#[test]
fn queued_text_normalizes_every_textual_wait_kind() {
    let session = start_input_project("@SYSTEM_TITLE\nINPUT\nRETURN\n");
    let original = session
        .operations
        .active_input()
        .expect("input wait")
        .clone();
    for (kind, expected) in [
        (WaitKind::EnterKey, VmValue::Integer(0)),
        (WaitKind::AnyKey, VmValue::Integer(0)),
        (WaitKind::IntegerValue, VmValue::Integer(42)),
        (WaitKind::StringValue, VmValue::String("42".into())),
        (WaitKind::AnyValue, VmValue::Integer(42)),
        (WaitKind::IntegerButton, VmValue::Integer(42)),
        (WaitKind::StringButton, VmValue::String("42".into())),
    ] {
        let mut pending = original.clone();
        pending.wait.kind = kind;
        if matches!(kind, WaitKind::IntegerButton | WaitKind::StringButton) {
            pending
                .choices
                .insert(InteractionToken { epoch: 1, id: 1 }, expected.clone());
        }
        let intent = queued_text_intent(&pending.wait, "42".into());
        assert_eq!(
            input_value(&pending, pending.wait.submission_token, intent, false),
            Some(InputSubmission::Value(expected)),
            "{kind:?}"
        );
    }
}

#[test]
fn one_input_uses_only_the_current_segment_and_empty_text_uses_the_default() {
    let session = start_input_project("@SYSTEM_TITLE\nINPUT\nRETURN\n");
    let mut pending = session
        .operations
        .active_input()
        .expect("input wait")
        .clone();
    pending.wait.one_input = true;
    pending.wait.default_value = Some(era_runtime_protocol::ProtocolValue::Integer(9));
    assert_eq!(
        input_value(
            &pending,
            pending.wait.submission_token,
            queued_text_intent(&pending.wait, String::new()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(9)))
    );
    assert_eq!(
        input_value(
            &pending,
            pending.wait.submission_token,
            queued_text_intent(&pending.wait, "12".into()),
            false,
        ),
        Some(InputSubmission::Value(VmValue::Integer(1)))
    );
}

#[test]
fn cancel_arbitrates_between_segments_and_renews_the_wait_identity() {
    let mut session = start_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nTWAIT 1000, 0\nINPUT\nSEEN = RESULT\nWAIT\nRETURN\n",
    );
    submit_text(&mut session, 3, "1\\n2");
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_wait(&session, WaitKind::EnterKey);
    assert_eq!(queued_text(&session), Some("2"));

    let old_wait = session
        .operations
        .active_input()
        .expect("timed wait")
        .wait
        .clone();
    submit_frontend_input(
        &mut session,
        4,
        old_wait.wait_id,
        InteractionToken {
            epoch: old_wait.submission_token.epoch,
            id: old_wait.submission_token.id + 1,
        },
        InputIntent::Cancel,
    );
    session.drive(single_transition_budget()).unwrap();
    assert_eq!(queued_text(&session), Some("2"));
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);

    submit_frontend_input(
        &mut session,
        5,
        old_wait.wait_id,
        old_wait.submission_token,
        InputIntent::Cancel,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let renewed = session
        .operations
        .active_input()
        .expect("renewed wait")
        .wait
        .clone();
    assert_ne!(renewed.wait_id, old_wait.wait_id);
    assert_eq!(renewed.deadline_ns, old_wait.deadline_ns);
    assert_eq!(
        renewed.countdown_remaining_ms,
        old_wait.countdown_remaining_ms
    );
    assert!(session.queued_input.is_empty());
    assert!(!session.message_skip);

    submit_frontend_input(
        &mut session,
        6,
        old_wait.wait_id,
        old_wait.submission_token,
        InputIntent::Enter,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);

    submit_frontend_input(
        &mut session,
        7,
        renewed.wait_id,
        renewed.submission_token,
        InputIntent::Enter,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    submit_text(&mut session, 8, "99");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 99);
}

#[test]
fn cancel_without_an_input_set_does_not_change_the_wait() {
    let mut session = start_input_project("@SYSTEM_TITLE\nINPUT\nWAIT\nRETURN\n");
    let wait = session
        .operations
        .active_input()
        .expect("input wait")
        .wait
        .clone();
    submit_frontend_input(
        &mut session,
        3,
        wait.wait_id,
        wait.submission_token,
        InputIntent::Cancel,
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::InvalidState);
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("unchanged wait")
            .wait,
        wait
    );
}

#[test]
fn rejected_and_recalled_inputs_do_not_leak_message_skip() {
    let mut session = start_input_project("@SYSTEM_TITLE\nINPUT\nWAIT\nRETURN\n");
    let wait = session
        .operations
        .active_input()
        .expect("input wait")
        .wait
        .clone();
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::CommitText("invalid".into()),
            message_skip: true,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::InvalidValue);
    assert!(!session.message_skip);

    session
        .negotiated_features
        .insert(RuntimeFeature::KeyMacros);
    session.key_macros.load("マクロキーF1:42");
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::ActivateKeyMacro { group: 0, slot: 0 },
            message_skip: true,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(session.text_box, "42");
    assert!(!session.message_skip);

    submit(
        &mut session,
        5,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: InteractionToken {
                epoch: wait.submission_token.epoch,
                id: wait.submission_token.id + 1,
            },
            monotonic_time_ns: 0,
            intent: InputIntent::CommitText("42".into()),
            message_skip: true,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);
    assert!(!session.message_skip);
    assert_eq!(input_replay_records(&session)[0]["step_count"], 0);
}

#[test]
fn new_input_and_undo_cannot_mutate_an_active_input_set() {
    let mut session =
        start_input_project("@SYSTEM_TITLE\nINPUT\nTWAIT 1000, 0\nINPUT\nWAIT\nRETURN\n");
    submit_text(&mut session, 3, "1\\n2");
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let wait = session
        .operations
        .active_input()
        .expect("timed wait")
        .wait
        .clone();

    submit_frontend_input(
        &mut session,
        4,
        wait.wait_id,
        wait.submission_token,
        InputIntent::CommitText("99".into()),
    );
    let epoch = session.epoch.0;
    submit(
        &mut session,
        5,
        RuntimeMessage::InputUndoRequest(InputUndoRequest {
            token: InteractionToken { epoch, id: 1 },
        }),
    );
    session.drive(single_transition_budget()).unwrap();
    let mut rejected = drain(&mut session)
        .into_iter()
        .filter_map(|message| match message {
            RuntimeMessage::CommandRejected(value) => Some(value.code),
            _ => None,
        })
        .collect::<Vec<_>>();
    session.drive(single_transition_budget()).unwrap();
    rejected.extend(
        drain(&mut session)
            .into_iter()
            .filter_map(|message| match message {
                RuntimeMessage::CommandRejected(value) => Some(value.code),
                _ => None,
            }),
    );
    assert_eq!(
        rejected,
        vec![
            CommandErrorCode::InvalidState,
            CommandErrorCode::InvalidState
        ]
    );
    assert_eq!(queued_text(&session), Some("2"));
}

#[test]
fn every_vm_snapshot_purpose_rejects_an_active_input_set() {
    let mut session =
        start_input_project("@SYSTEM_TITLE\nINPUT\nTWAIT 1000, 0\nINPUT\nWAIT\nRETURN\n");
    session
        .negotiated_features
        .insert(RuntimeFeature::VmSnapshot);
    submit_text(&mut session, 3, "1\\n2");
    session.drive(RuntimeDriveBudget::default()).unwrap();

    for (index, purpose) in [
        SnapshotExportPurpose::Normal,
        SnapshotExportPurpose::Debug,
        SnapshotExportPurpose::Diagnosis,
    ]
    .into_iter()
    .enumerate()
    {
        submit(
            &mut session,
            4 + index as u64,
            RuntimeMessage::StateExportRequest(StateExportRequest {
                kind: StateExportKind::VmSnapshot,
                snapshot_purpose: purpose,
            }),
        );
        session.drive(single_transition_budget()).unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(
            message,
            RuntimeMessage::StateExportReady(StateExportReady {
                result: StateExportResult::Ineligible { reasons },
                ..
            }) if reasons.contains(&SnapshotIneligibleReason::SnapshotStateUnavailable)
        )));
    }
}

pub(super) fn start_input_project(source: &str) -> RuntimeSession {
    start_input_project_with(
        source,
        era_runtime_protocol::CompatibilityIdentity::default(),
        capabilities(),
        false,
    )
}

fn start_snake_input_project(source: &str, client: ClientCapabilities) -> RuntimeSession {
    start_input_project_with(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        client,
        false,
    )
}

fn start_input_project_with(
    source: &str,
    identity: era_runtime_protocol::CompatibilityIdentity,
    client: ClientCapabilities,
    undo: bool,
) -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "input-set-test".into(),
            features: if undo {
                vec![RuntimeFeature::TimedInput, RuntimeFeature::InputUndo]
            } else {
                vec![RuntimeFeature::TimedInput]
            },
            requested_limits: RuntimeOptions::default().limits,
            capabilities: client,
            preferred_locales: vec!["en".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let mut config = profile_configuration_file(identity.profile);
    if undo && let FilePayload::Utf8(text) = &mut config.payload {
        text.push_str("[input]\nundo_enabled = true\n");
    }
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: identity,
            project_revision: 1,
            files: vec![
                config,
                SubmittedFile {
                    relative_path: "input-set.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
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
    drive_until_wait(&mut session);
    session
}

fn submit_text(session: &mut RuntimeSession, sequence: u64, text: &str) {
    let pending = session.operations.active_input().expect("active input");
    submit_frontend_input(
        session,
        sequence,
        pending.wait.wait_id,
        pending.wait.submission_token,
        InputIntent::CommitText(text.into()),
    );
}

fn submit_frontend_input(
    session: &mut RuntimeSession,
    sequence: u64,
    wait_id: u64,
    token: InteractionToken,
    intent: InputIntent,
) {
    submit(
        session,
        sequence,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            monotonic_time_ns: 0,
            intent,
            message_skip: false,
        }),
    );
}

fn drive_input_set(session: &mut RuntimeSession) {
    for _ in 0..128 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput && session.queued_input.is_empty() {
            return;
        }
    }
    panic!("input set did not reach its final wait");
}

fn drive_until_wait(session: &mut RuntimeSession) {
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.phase == RuntimePhase::WaitingInput
            && session.operations.active_input().is_some()
        {
            return;
        }
    }
    panic!("project did not reach its first wait");
}

fn assert_wait(session: &RuntimeSession, kind: WaitKind) {
    assert_eq!(
        session
            .operations
            .active_input()
            .map(|pending| pending.wait.kind),
        Some(kind)
    );
}

fn queued_text(session: &RuntimeSession) -> Option<&str> {
    session
        .queued_input
        .front()
        .map(|segment| segment.text.as_str())
}

fn assert_rejection(session: &mut RuntimeSession, code: CommandErrorCode) {
    assert!(drain(session).iter().any(|message| matches!(
        message,
        RuntimeMessage::CommandRejected(value) if value.code == code
    )));
}

fn single_transition_budget() -> RuntimeDriveBudget {
    RuntimeDriveBudget {
        maximum_runtime_transitions: 1,
        ..RuntimeDriveBudget::default()
    }
}

fn runtime_integer(session: &RuntimeSession, name: &str) -> i64 {
    read_runtime_integer(session.vm.as_ref().expect("runtime VM"), name, &[], None).unwrap()
}

fn runtime_string(session: &RuntimeSession, name: &str) -> String {
    read_runtime_string(session.vm.as_ref().expect("runtime VM"), name).unwrap()
}

fn snake_input_capabilities() -> ClientCapabilities {
    let mut client = capabilities();
    client.environment = [
        INPUT_DEVICE_LATCH_CAPABILITY,
        INPUT_DEVICE_PUMP_CAPABILITY,
        INPUT_TIMED_VIEWPORT_CAPABILITY,
    ]
    .into_iter()
    .map(|name| era_runtime_protocol::EnvironmentCapability {
        name: name.into(),
        versions: VersionRange::exact(era_runtime_protocol::INPUT_ENVIRONMENT_VERSION),
    })
    .collect();
    client.services.push(ServiceCapability {
        kind: ServiceKind::InputState,
        operation: DEVICE_PUMP_OPERATION.into(),
        versions: VersionRange::exact(DEVICE_PUMP_OPERATION_VERSION),
    });
    client
}

fn send_focus(session: &mut RuntimeSession, sequence: u64, focused: bool) {
    submit(
        session,
        sequence,
        RuntimeMessage::ClientStateChanged(era_runtime_protocol::ClientStateChanged {
            focused,
            visible: true,
            audio_available: false,
            reduce_motion: false,
            high_contrast: false,
            screen_reader: false,
        }),
    );
}
fn send_key(session: &mut RuntimeSession, sequence: u64, event_sequence: u64, pressed: bool) {
    submit(
        session,
        sequence,
        RuntimeMessage::DeviceStateChanged(era_runtime_protocol::DeviceStateChanged {
            device: era_runtime_protocol::InputDeviceKind::Keyboard,
            code: 65,
            pressed,
            event_sequence,
            toggle: false,
            repeat: false,
            x: 0,
            y: 0,
            monotonic_time_ns: event_sequence,
        }),
    );
}

#[test]
fn script_sequence_precedes_but_preserves_previously_expanded_macro_tails() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM FIRST\n#DIM SECOND\nINPUT\nRESULT = SEQUENCEINPUT(\"21\")\nINPUT\nFIRST = RESULT\nINPUT\nSECOND = RESULT\nFORCEWAIT\nRETURN\n",
        capabilities(),
    );
    submit_text(&mut session, 3, "1\\n90");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "FIRST"), 21);
    assert_eq!(runtime_integer(&session, "SECOND"), 90);
    assert!(session.input_controller.pending_sequence.is_none());
    let records = input_replay_records(&session);
    assert_eq!(records[0]["step_count"], 3);
    assert_eq!(records[2]["source"]["raw"], "21");
    assert_eq!(records[3]["source"]["fragment"], 1);
}

#[test]
fn script_sequence_last_write_wins_including_the_empty_string() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIMS SEEN\nINPUT\nRESULT = SEQUENCEINPUT(\"discard\")\nRESULT = SEQUENCEINPUT(\"\")\nINPUTS\nSEEN '= RESULTS\nFORCEWAIT\nRETURN\n",
        capabilities(),
    );
    submit_text(&mut session, 3, "1");
    drive_input_set(&mut session);
    assert_eq!(runtime_string(&session, "SEEN"), "");
    assert!(session.input_controller.pending_sequence.is_none());
    assert_eq!(input_replay_records(&session)[2]["source"]["raw"], "");
}

#[test]
fn disabling_macro_preserves_admitted_tails_and_admits_new_text_as_one_literal() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM FIRST\n#DIMS SEEN\nINPUT\nRESULT = DISABLE_INPUT_MACRO()\nINPUTS\nFIRST = TOINT(RESULTS)\nINPUTS\nSEEN '= RESULTS\nFORCEWAIT\nRETURN\n",
        capabilities(),
    );
    submit_text(&mut session, 3, "1\\n2");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "FIRST"), 2);
    let literal = "@CONFIG(1\\n2)*2\\e";
    submit_text(&mut session, 4, literal);
    drive_input_set(&mut session);
    assert_eq!(runtime_string(&session, "SEEN"), literal);
    assert!(session.queued_input.is_empty());
    assert!(!session.message_skip);
}

#[test]
fn inactive_compiled_and_form_key_queries_skip_argument_and_preserve_latch() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nRESULT = GETKEY(BUMP())\nRESULT = STRFORMCHECK(\"{GETKEYTRIGGERED(BUMP())}\")\nINPUT\nSEEN = TOINT(STRFORM(\"{GETKEYTRIGGERED(65)}\"))\nFORCEWAIT\nRETURN\n@BUMP\n#FUNCTION\nFLAG:0 += 1\nRETURNF 65\n",
        snake_input_capabilities(),
    );
    send_key(&mut session, 3, 1, true);
    send_key(&mut session, 4, 2, false);
    send_focus(&mut session, 5, false);
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let inactive_devices = session.device_input.clone();
    submit_text(&mut session, 6, "0");
    drive_input_set(&mut session);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        0
    );
    assert_eq!(session.device_input, inactive_devices);
    send_focus(&mut session, 7, true);
    submit_text(&mut session, 8, "1");
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 1);
    assert_eq!(session.device_input.snake_query(65, true), 0);
}

#[test]
fn unavailable_latch_skips_key_arguments_and_warns_once_per_execution_site() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\nFOR LOCAL, 0, 2\nRESULT = GETKEY(BUMP())\nNEXT\nWAIT\nRETURN\n@BUMP\n#FUNCTION\nFLAG:0 += 1\nRETURNF 65\n",
        capabilities(),
    );
    assert_eq!(runtime_integer(&session, "RESULT"), 0);
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        0
    );
    let notices = drain(&mut session)
        .into_iter()
        .filter(|message| {
            matches!(message,
            RuntimeMessage::Diagnostic(value)
                if value.code == "compat.input.device_latch_unavailable" && value.source.is_some())
        })
        .count();
    assert_eq!(notices, 1);
}

#[test]
fn await_zero_requires_ack_and_only_new_ordered_events_survive_the_pump() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nAWAIT 0\nSEEN = GETKEYTRIGGERED(65)\nFORCEWAIT\nRETURN\n",
        snake_input_capabilities(),
    );
    send_focus(&mut session, 3, true);
    send_key(&mut session, 4, 1, true);
    send_key(&mut session, 5, 2, false);
    submit_text(&mut session, 6, "0");
    let mut pump = None;
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(&mut session) {
            if let RuntimeMessage::ServiceRequest(request) = message
                && request.operation == DEVICE_PUMP_OPERATION
            {
                pump = Some(request);
            }
        }
        if pump.is_some() {
            break;
        }
    }
    let pump = pump.expect("AWAIT 0 must emit a device pump request");
    let request: DevicePumpRequest =
        era_protocol::decode_canonical(pump.payload.as_slice()).unwrap();
    assert_eq!(request.after_event_sequence, 2);
    session
        .negotiated_features
        .insert(RuntimeFeature::VmSnapshot);
    for purpose in [
        SnapshotExportPurpose::Normal,
        SnapshotExportPurpose::Debug,
        SnapshotExportPurpose::Diagnosis,
    ] {
        session
            .export_state(
                0,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: purpose,
                },
            )
            .unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(message,
            RuntimeMessage::StateExportReady(StateExportReady { result: StateExportResult::Ineligible { reasons }, .. })
            if reasons.contains(&SnapshotIneligibleReason::SnapshotStateUnavailable))));
    }
    assert_eq!(runtime_integer(&session, "SEEN"), 0);
    assert_eq!(session.device_input.snake_query(65, true), 0);
    send_key(&mut session, 7, 3, true);
    send_key(&mut session, 8, 4, false);
    submit(
        &mut session,
        9,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: pump.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&DevicePumpResponse {
                        epoch: request.epoch,
                        through_event_sequence: 4,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 1);
    send_key(&mut session, 10, 3, true);
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);
    assert_eq!(session.device_input.snake_query(65, true), 0);
}

#[test]
fn admission_queued_before_epoch_change_is_rejected_when_drained() {
    let mut session = start_input_project("@SYSTEM_TITLE\nINPUT\nRETURN\n");
    send_key(&mut session, 3, 1, true);
    session.advance_epoch();
    session.drive(single_transition_budget()).unwrap();
    assert_rejection(&mut session, CommandErrorCode::StaleRequest);
    assert_eq!(session.device_input.event_sequence, 0);
}

#[test]
fn environment_negotiation_controls_platform_value_without_claiming_a_host_os() {
    for (client, platform, known) in [(capabilities(), 5, 0), (snake_input_capabilities(), 0, 1)] {
        let mut session = start_snake_input_project(
            "@SYSTEM_TITLE\n#DIM PLATFORM\n#DIM KNOWN\n#DIM ZERO\n#DIM NEGATIVE\nFOR LOCAL, 0, 2\nPLATFORM = GETPLATFORM()\nNEXT\nKNOWN = ENV_HAS_CAPABILITY(\"input.timed_viewport\")\nZERO = ENV_HAS_CAPABILITY(\"input.timed_viewport\", 0)\nNEGATIVE = ENV_HAS_CAPABILITY(\"input.timed_viewport\", -1)\nRESULT = ENV_HAS_CAPABILITY(\"unknown.capability\")\nWAIT\nRETURN\n",
            client,
        );
        assert_eq!(runtime_integer(&session, "PLATFORM"), platform);
        assert_eq!(runtime_integer(&session, "KNOWN"), known);
        assert_eq!(runtime_integer(&session, "ZERO"), 0);
        assert_eq!(runtime_integer(&session, "NEGATIVE"), 0);
        assert_eq!(runtime_integer(&session, "RESULT"), 0);
        let messages = drain(&mut session);
        let notices: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                RuntimeMessage::Diagnostic(value)
                    if value.code == "compat.portability.platform_mapping" =>
                {
                    Some(value)
                }
                _ => None,
            })
            .collect();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].source.is_some());
    }
}

#[test]
fn sequence_waiting_for_admission_rejects_all_snapshot_purposes() {
    let mut session = start_snake_input_project("@SYSTEM_TITLE\nINPUT\nRETURN\n", capabilities());
    let vm = session.vm.as_ref().unwrap();
    session.input_controller.pending_sequence = Some(PendingSequence {
        text: String::new(),
        site: SequenceSite {
            artifact: vm.artifact_id(),
            function: vm.vm().artifact().functions[0].key,
            instruction: 0,
        },
    });
    for purpose in [
        SnapshotExportPurpose::Normal,
        SnapshotExportPurpose::Debug,
        SnapshotExportPurpose::Diagnosis,
    ] {
        session
            .export_state(
                0,
                StateExportRequest {
                    kind: StateExportKind::VmSnapshot,
                    snapshot_purpose: purpose,
                },
            )
            .unwrap();
        assert!(drain(&mut session).iter().any(|message| matches!(message,
            RuntimeMessage::StateExportReady(StateExportReady { result: StateExportResult::Ineligible { reasons }, .. })
            if reasons.contains(&SnapshotIneligibleReason::SnapshotStateUnavailable))));
    }
    assert!(session.input_controller.pending_sequence.is_some());
}

#[test]
fn undo_regenerates_script_sequence_without_injecting_a_second_copy_and_restores_macro_switch() {
    let source = concat!(
        "@SYSTEM_TITLE\nCALL SHARED_INPUT\nRETURN\n@SHOW_SHOP\nCALL SHARED_INPUT\nRETURN\n",
        "@SHARED_INPUT\nINPUTS\nSAVESTR:0 '= RESULTS\nRESULT = SEQUENCEINPUT(\"script\")\nINPUTS\nSAVESTR:1 '= RESULTS\nINPUTS\nWAIT\nRETURN\n"
    );
    let mut session = start_input_project_with(
        source,
        erabasic_compat::CompatibilityIdentity::for_profile(
            erabasic_compat::CompatibilityProfileId::EmueraSkiaSnake,
        ),
        capabilities(),
        true,
    );
    let random = session.vm.as_ref().unwrap().export_random_state().unwrap();
    let baseline = {
        let vm = session.vm.as_ref().unwrap();
        session
            .encode_owned_runtime_save(
                vm,
                "input checkpoint".into(),
                Vec::new(),
                session.traditional_save_format(),
            )
            .unwrap()
    };
    session
        .establish_input_undo_checkpoint(3, baseline, random)
        .unwrap();
    submit_text(&mut session, 3, "external");
    drive_input_set(&mut session);
    assert_eq!(session.undo_checkpoint.as_ref().unwrap().inputs.len(), 2);
    submit_text(&mut session, 4, "remove");
    drive_input_set(&mut session);
    session.input_controller.macro_enabled = false;
    let token = session.input_undo_state().token.expect("undo token");
    submit(
        &mut session,
        5,
        RuntimeMessage::InputUndoRequest(InputUndoRequest { token }),
    );
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.undo_replay.is_none() && session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert_wait(&session, WaitKind::StringValue);
    assert!(session.input_controller.macro_enabled);
    assert!(session.input_controller.pending_sequence.is_none());
    assert!(session.queued_input.is_empty());
    let checkpoint = session.undo_checkpoint.as_ref().unwrap();
    assert_eq!(
        checkpoint
            .inputs
            .iter()
            .map(|input| input.value.as_str())
            .collect::<Vec<_>>(),
        ["external", "script"]
    );
    assert!(matches!(
        checkpoint.inputs[1].source.as_ref().unwrap().root,
        InputRoot::Sequence(_)
    ));
    assert_eq!(
        input_replay_records(&session)[0]["step_count"],
        0,
        "automatic revalidation is not a new frontend admission"
    );
}

#[test]
fn await_positive_duration_starts_after_ack_not_at_request_creation() {
    let mut session = start_snake_input_project(
        "@SYSTEM_TITLE\n#DIM SEEN\nINPUT\nAWAIT 10\nSEEN = 1\nFORCEWAIT\nRETURN\n",
        snake_input_capabilities(),
    );
    submit_text(&mut session, 3, "0");
    let mut pump = None;
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        for message in drain(&mut session) {
            if let RuntimeMessage::ServiceRequest(request) = message
                && request.operation == DEVICE_PUMP_OPERATION
            {
                pump = Some(request);
            }
        }
        if pump.is_some() {
            break;
        }
    }
    let pump = pump.expect("device pump");
    let request: DevicePumpRequest =
        era_protocol::decode_canonical(pump.payload.as_slice()).unwrap();
    session.observe_frontend_time(0);
    submit(
        &mut session,
        4,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 100_000_000,
        }),
    );
    submit(
        &mut session,
        5,
        RuntimeMessage::ServiceResponse(ServiceResponse {
            request_id: pump.request_id,
            result: ServiceResult::Ready {
                payload: ProtocolBytes::new(
                    encode_canonical(&DevicePumpResponse {
                        epoch: request.epoch,
                        through_event_sequence: request.after_event_sequence,
                    })
                    .unwrap(),
                ),
            },
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "SEEN"), 0);
    submit(
        &mut session,
        6,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 109_999_999,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    assert_eq!(runtime_integer(&session, "SEEN"), 0);
    submit(
        &mut session,
        7,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 110_000_000,
        }),
    );
    drive_input_set(&mut session);
    assert_eq!(runtime_integer(&session, "SEEN"), 1);
}
