use super::*;

#[test]
#[allow(clippy::too_many_lines)]
fn restart_redraws_string_and_integer_button_menus_in_the_current_function() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "restart-menu-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("hello");
    drain(&mut session);
    let source = concat!(
        "@SYSTEM_TITLE\nCALL ORACLE_RESTART_FLOW\nWAIT\nRETURN\n",
        include_str!("../../../../../tools/runtime-tester/fixture-reference/erb/restart.erb")
    );
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "restart.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(
            |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        ),
        "project load failed: {loaded:?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("move wait");
        if session.operations.active_input().is_some() {
            break;
        }
    }

    let (wait_id, submission_token, c_button) = {
        let pending = session.operations.active_input().expect("move menu wait");
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::String("C".into())).then_some(*token))
            .expect("C button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .last()
            .is_some_and(|line| line.line_end),
        "INPUTS must flush the button row before opening its wait"
    );
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(c_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("restart move menu");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let (wait_id, submission_token, zero_button) = {
        let pending = session
            .operations
            .active_input()
            .expect("restarted move menu wait");
        assert_eq!(pending.wait.kind, WaitKind::StringValue);
        assert!(
            pending
                .choices
                .values()
                .any(|value| *value == VmValue::String("C".into()))
        );
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::String("0".into())).then_some(*token))
            .expect("move return button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .last()
            .is_some_and(|line| line.line_end),
        "restarted INPUTS must not reuse the previous menu row"
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(zero_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("ability wait");
        if session
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.kind == WaitKind::IntegerValue)
        {
            break;
        }
    }

    let (wait_id, submission_token, next_page_button) = {
        let pending = session
            .operations
            .active_input()
            .expect("ability menu wait");
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::Integer(6)).then_some(*token))
            .expect("next page button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    submit(
        &mut session,
        5,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(next_page_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("restart ability menu");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let pending = session
        .operations
        .active_input()
        .expect("restarted ability menu wait");
    assert_eq!(pending.wait.kind, WaitKind::IntegerValue);
    assert!(
        pending
            .choices
            .values()
            .any(|value| *value == VmValue::Integer(6))
    );

    let snapshot = session.presentation.snapshot();
    let visible_text = projected_presentation_text(&snapshot);
    assert!(visible_text.contains("move display=1"), "{visible_text}");
    assert!(visible_text.contains("ability page=1"), "{visible_text}");
    assert!(!visible_text.contains("invalid move"), "{visible_text}");
    assert!(!visible_text.contains("invalid ability"), "{visible_text}");
}

#[test]
fn inputs_accepts_an_automatic_button_from_the_pending_print_buffer() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "pending-auto-button-test".into(),
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
            files: vec![SubmittedFile {
                relative_path: "pending-button.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    concat!(
                        "@SYSTEM_TITLE\nCALL ORACLE_PENDING_AUTO_BUTTON\nWAIT\nRETURN\n",
                        include_str!(
                            "../../../../../tools/runtime-tester/fixture-reference/erb/restart.erb"
                        )
                    )
                    .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(
            |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        ),
        "project load failed: {loaded:?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("input wait");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let (wait_id, submission_token, back_button) = {
        let pending = session.operations.active_input().expect("INPUTS wait");
        assert_eq!(pending.wait.kind, WaitKind::StringValue);
        let token = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::Integer(58)).then_some(*token))
            .expect("pending automatic button must belong to the active wait");
        (pending.wait.wait_id, pending.wait.submission_token, token)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(back_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("accept back button");
    }
    assert!(
        session
            .presentation
            .snapshot()
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("pending auto=58"))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn visible_buttons_from_an_earlier_wait_remain_usable_until_breakbutton() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "visible-old-button-test".into(),
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
            files: vec![SubmittedFile {
                relative_path: "old-button.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nPRINTBUTTON \"[阿燐]\", 1036\nPRINTL\nINPUT\nPRINT [100] - 返回\nINPUT\nPRINTFORML selected={RESULT}\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(
            |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        ),
        "project load failed: {loaded:?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("first input");
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let (first_wait, first_submission, character_button) = {
        let pending = session.operations.active_input().expect("first wait");
        let button = pending
            .choices
            .iter()
            .find_map(|(token, value)| (*value == VmValue::Integer(1036)).then_some(*token))
            .expect("character button");
        (pending.wait.wait_id, pending.wait.submission_token, button)
    };
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: first_wait,
            token: first_submission,
            monotonic_time_ns: 0,
            intent: InputIntent::Activate(character_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("second input");
        if session
            .operations
            .active_input()
            .is_some_and(|pending| pending.wait.wait_id != first_wait)
        {
            break;
        }
    }
    let (second_wait, second_submission) = {
        let pending = session.operations.active_input().expect("second wait");
        assert!(
            !pending.choices.contains_key(&character_button),
            "the earlier token must exercise visible-history fallback"
        );
        (pending.wait.wait_id, pending.wait.submission_token)
    };
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id: second_wait,
            token: second_submission,
            monotonic_time_ns: 1,
            intent: InputIntent::Activate(character_button),
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("accept earlier button");
    }
    let visible = session
        .presentation
        .snapshot()
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
        .collect::<Vec<_>>()
        .join("");
    assert!(visible.contains("selected=1036"), "{visible}");
}

#[test]
fn skipdisp_silently_skips_wait_commands_like_the_reference() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "skipdisp-wait-test".into(),
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
            files: vec![SubmittedFile {
                relative_path: "skipdisp.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nSKIPDISP 1\nWAIT\nWAITANYKEY\nFORCEWAIT\nTWAIT 1, 0\nSKIPDISP 0\nPRINTL visible\nWAIT\nRETURN\n"
                        .into(),
                ),
                content_hash: None,
            }],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(
            |message| matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        ),
        "project load failed: {loaded:?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        if session.operations.active_input().is_some() || session.phase == RuntimePhase::Faulted {
            break;
        }
    }
    assert_ne!(session.phase, RuntimePhase::Faulted);
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("only the final WAIT should open")
            .wait
            .kind,
        WaitKind::EnterKey
    );
    let visible = session
        .presentation
        .snapshot()
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
        .collect::<Vec<_>>()
        .join("");
    assert!(visible.contains("visible"), "{visible}");
}

#[test]
#[allow(clippy::too_many_lines)]
fn input_undo_records_only_accepted_scalar_input_after_a_checkpoint() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "undo-test".into(),
            features: vec![RuntimeFeature::InputUndo],
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
            files: vec![
                SubmittedFile {
                    relative_path: "emuera.config".into(),
                    category: FileCategory::Configuration,
                    payload: FilePayload::Utf8("Enable undo with ctrl-z:YES\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nINPUT\nWAIT\nRETURN\n@SHOW_SHOP\nWAIT\nRETURN\n".into(),
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
            mode: StartMode::NewGame { seed: Some(7) },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let random = session.vm.as_ref().unwrap().export_random_state().unwrap();
    let baseline = {
        let vm = session.vm.as_ref().unwrap();
        encode_scoped_save(
            &vm.export_era_state(),
            vm.vm().artifact(),
            era_runtime_save::SaveFileKind::Normal,
            "checkpoint".into(),
            Vec::new(),
            session.traditional_save_format(),
        )
        .unwrap()
    };
    session
        .establish_input_undo_checkpoint(3, baseline, random)
        .unwrap();
    let (wait_id, token) = session
        .operations
        .active_input()
        .map(|pending| (pending.wait.wait_id, pending.wait.submission_token))
        .unwrap();
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token,
            intent: InputIntent::CommitText("42".into()),
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(session.undo_checkpoint.as_ref().unwrap().inputs, vec!["42"]);
    let state = session.input_undo_state();
    assert!(state.enabled);
    assert_eq!(state.available_steps, 1);
    let undo_token = state.token.expect("undo token");
    submit(
        &mut session,
        4,
        RuntimeMessage::InputUndoRequest(InputUndoRequest { token: undo_token }),
    );
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.undo_replay.is_none() && session.operations.active_input().is_some() {
            break;
        }
    }
    assert_ne!(session.phase, RuntimePhase::Faulted);
    assert!(session.undo_checkpoint.as_ref().unwrap().inputs.is_empty());
    assert_eq!(session.input_undo_state().available_steps, 0);
    let records = input_replay_records(&session);
    assert_eq!(records[0]["origin"]["kind"], "input_undo");
    assert_eq!(records[0]["origin"]["retained_input_count"], 0);
    assert_eq!(records[0]["step_count"], 0);
    assert_eq!(
        records.len(),
        1,
        "automatic Ctrl-Z replay must not write steps"
    );
    let wait = session
        .operations
        .active_input()
        .expect("post-undo wait")
        .wait
        .clone();
    submit(
        &mut session,
        5,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 1,
            intent: InputIntent::Enter,
            message_skip: false,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let records = input_replay_records(&session);
    assert_eq!(records[0]["step_count"], 1);
    assert_eq!(records[1]["action"], "enter");
}

#[test]
fn input_undo_keeps_the_next_scalar_queued_across_primitive_waits() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.undo_replay = Some(UndoReplay {
        remaining: VecDeque::from(["12".to_owned()]),
        queued_repeats: 0,
    });
    let mut wait = session.system_wait(InteractionToken { epoch: 0, id: 1 });
    wait.kind = WaitKind::PrimitiveMouseKey;
    assert_eq!(session.replay_submission(&wait), None);
    assert_eq!(
        session.undo_replay.as_ref().unwrap().remaining,
        VecDeque::from(["12".to_owned()])
    );

    wait.kind = WaitKind::IntegerValue;
    assert_eq!(
        session.replay_submission(&wait),
        Some(InputSubmission::Value(VmValue::Integer(12)))
    );
    assert!(session.undo_replay.as_ref().unwrap().remaining.is_empty());
}

#[test]
fn autosave_failure_prints_both_reference_messages_and_waits_before_shop() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    session.selected_locale = "en".into();
    session.stage_builtin_autosave_failure().unwrap();
    assert_eq!(session.controller.step, SystemStep::ShopAutosaveFailureWait);
    assert_eq!(session.phase, RuntimePhase::WaitingInput);
    assert_eq!(
        session.operations.active_input().unwrap().wait.kind,
        WaitKind::EnterKey
    );
    let keys = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .into_iter()
        .flat_map(|line| line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text {
                system_text: Some(reference),
                ..
            }
            | era_runtime_protocol::DisplayRun::TextLayout {
                system_text: Some(reference),
                ..
            } => Some(reference.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            SystemTextKey::AutoSaveFailed,
            SystemTextKey::AutoSaveSkipped
        ]
    );
}

#[test]
fn stopcalltrain_discards_its_caller_and_resumes_the_train_system_phase() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "continuous-test".into(),
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
                        "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN RESULT\n@SHOW_USERCOM\n#DIM ONCE\nIF ONCE == 0\nSELECTCOM:1 = 0\nONCE = 1\nCALLTRAIN 1\nENDIF\nRETURN\n@COM0\nSTOPCALLTRAIN\nRESULT:30 = 1\nRETURN\n@SOURCE_CHECK\nRETURN\n@CALLTRAINEND\nRESULT:31 = 1\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "TRAIN.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("0,go\n".into()),
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
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:?}");
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[30], None).unwrap(),
        0,
        "the STOPCALLTRAIN caller must not resume"
    );
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[31], None).unwrap(),
        1
    );
    assert_eq!(session.controller.step, SystemStep::TrainEventComEndWait);
    assert_eq!(
        session.operations.active_input().unwrap().wait.kind,
        WaitKind::EnterKey
    );
}

#[test]
fn continuous_train_reports_progress_and_routes_unavailable_commands_to_usercom() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "continuous-output-test".into(),
            features: Vec::new(),
            requested_limits: RuntimeOptions::default().limits,
            capabilities: capabilities(),
            preferred_locales: vec!["ja".into()],
            configuration_profile: None,
        }),
    );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    drain(&mut session);
    let source = "@SYSTEM_TITLE\nBEGIN TRAIN\n@EVENTTRAIN\nRETURN\n@SHOW_STATUS\nRETURN\n@COM_ABLE0\nRESULT = 1\nRETURN\n@SHOW_USERCOM\n#DIM ONCE\nIF ONCE == 0\nSELECTCOM:1 = 1\nCALLTRAIN 1\nONCE = 1\nENDIF\nRETURN\n@USERCOM\nRETURN\n@CALLTRAINEND\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "TRAIN.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("0,go\n".into()),
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
    for _ in 0..64 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let output = drain(&mut session);
    assert_ne!(session.phase, RuntimePhase::Faulted, "{output:?}");
    let keys = session
        .presentation
        .snapshot()
        .history
        .logical_lines
        .into_iter()
        .flat_map(|line| line.runs)
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text {
                system_text: Some(reference),
                ..
            }
            | era_runtime_protocol::DisplayRun::TextLayout {
                system_text: Some(reference),
                ..
            } => Some(reference.key),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(keys.contains(&SystemTextKey::ContinuousTrainProgress));
    assert!(keys.contains(&SystemTextKey::ContinuousTrainCommandFailed));
    assert!(!session.controller.continuous_train);
}
