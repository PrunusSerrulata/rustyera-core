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

fn start_input_project(source: &str) -> RuntimeSession {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "input-set-test".into(),
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
                relative_path: "input-set.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
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
