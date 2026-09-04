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
        include_str!("../../../../../../tools/runtime-tester/fixture-reference/erb/restart.erb")
    );
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "pending-button.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    concat!(
                        "@SYSTEM_TITLE\nCALL ORACLE_PENDING_AUTO_BUTTON\nWAIT\nRETURN\n",
                        include_str!(
                            "../../../../../../tools/runtime-tester/fixture-reference/erb/restart.erb"
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
            compatibility: era_runtime_protocol::CompatibilityIdentity::default(),
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
