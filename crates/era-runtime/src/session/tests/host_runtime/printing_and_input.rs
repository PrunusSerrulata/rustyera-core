#[test]
#[allow(clippy::too_many_lines)]
fn printform_and_printc_family_preserve_reference_semantics() {
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

    let source = "@SYSTEM_TITLE\nLOCALS:0 = 你\nPRINTFORML %LOCALS:0,20,LEFT%体\nLOCALS:0 = 霊夢\nPRINTFORML %LOCALS:0,20,LEFT%体\nCALL ORACLE_PRINT_FAMILY\nWAIT\nRETURN\n";
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
                    payload: FilePayload::Utf8(source.into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "print-family.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        include_str!(
                            "../../../../../../tools/runtime-tester/fixture-reference/erb/print-family.erb"
                        )
                        .into(),
                    ),
                    content_hash: None,
                },
            ],
        }),
    );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    let load = drain(&mut session);
    assert!(
        load.iter().any(|message| matches!(
            message,
            RuntimeMessage::ProjectLoadReport(report) if report.success
        )),
        "{load:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    let mut run_messages = Vec::new();
    for _ in 0..32 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        run_messages.extend(drain(&mut session));
        if session.phase() == RuntimePhase::WaitingInput {
            break;
        }
    }
    assert_eq!(
        session.phase(),
        RuntimePhase::WaitingInput,
        "{run_messages:#?}"
    );
    let snapshot = session.presentation.snapshot();

    let rendered = snapshot
        .history
        .logical_lines
        .iter()
        .map(|line| flattened_display_text(&line.runs))
        .collect::<Vec<_>>();
    assert!(
        rendered.contains(&format!("你{}体", " ".repeat(18))),
        "{rendered:#?}"
    );
    assert!(
        rendered.contains(&format!("霊夢{}体", " ".repeat(16))),
        "{rendered:#?}"
    );
    assert!(
        rendered.contains(&"|  7|7  |界  |Target|Call|Call|Target|Call| X".into()),
        "{rendered:#?}"
    );
    assert!(rendered.contains(&"ヒラガナ".into()), "{rendered:#?}");

    let cell_line = snapshot
        .history
        .logical_lines
        .iter()
        .find(|line| {
            line.runs
                .iter()
                .filter(|run| matches!(run, DisplayRun::ColumnCell { .. }))
                .count()
                == 4
        })
        .expect("four script PRINTC cells must remain on one line");
    let cells = cell_line
        .runs
        .iter()
        .filter_map(|run| match run {
            DisplayRun::ColumnCell {
                content,
                alignment,
                width,
            } => Some((content, alignment, width)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 4);
    assert_eq!(*cells[0].1, era_runtime_protocol::CellAlignment::Right);
    assert_eq!(*cells[1].1, era_runtime_protocol::CellAlignment::Left);
    assert!(cells.iter().all(|cell| {
        *cell.2 == era_runtime_protocol::CellWidthIntent::ProjectColumns(24)
    }));
    assert!(
        cells
            .iter()
            .all(|cell| matches!(cell.0.as_slice(), [DisplayRun::Button { .. }]))
    );
    let DisplayRun::Button { runs, .. } = &cells[0].0[0] else {
        unreachable!()
    };
    let (DisplayRun::Text { style, .. } | DisplayRun::TextLayout { style, .. }) = &runs[0] else {
        unreachable!()
    };
    assert_eq!(style.foreground.red, 0xc0);
    assert_eq!(
        session
            .operations
            .active_input()
            .expect("WAIT")
            .choices
            .len(),
        4
    );
}

#[test]
fn queued_timer_precedes_later_input_and_rejects_its_expired_token() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TimedInput],
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
                    relative_path: "input.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\nTINPUT 1000, 7, 1, \"timeout\", 0, 0\nTINPUT 1000, 9, 1, \"timeout\", 0, 0\nPRINTFORML got={RESULT}\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).expect("load");
    drain(&mut session);
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..4 {
        session.drive(RuntimeDriveBudget::default()).expect("wait");
    }
    let opened = drain(&mut session);
    let wait = opened
        .iter()
        .find_map(|message| match message {
            RuntimeMessage::WaitChanged(WaitChange::Opened(wait)) => Some(wait.clone()),
            _ => None,
        })
        .expect("runtime should publish the input wait");
    assert_eq!(
        wait.default_value,
        Some(era_runtime_protocol::ProtocolValue::Integer(7))
    );
    assert_eq!(wait.stability, WaitStability::Transient);

    session.observe_frontend_time(0);
    submit(
        &mut session,
        3,
        RuntimeMessage::AdvanceTime(AdvanceTime {
            monotonic_time_ns: 2_000_000_000,
        }),
    );
    submit(
        &mut session,
        4,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 2_000_000_000,
            intent: InputIntent::CommitText("42".into()),
            message_skip: true,
        }),
    );
    for _ in 0..4 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("resume");
    }
    let messages = drain(&mut session);
    assert!(messages.iter().any(|message| matches!(message,
        RuntimeMessage::CommandRejected(error) if error.code == CommandErrorCode::StaleRequest)));
    assert_eq!(read_runtime_integer(session.vm.as_ref().unwrap(), "RESULT", &[], None).unwrap(), 7);
    assert_eq!(read_runtime_integer(session.vm.as_ref().unwrap(), "ISTIMEOUT", &[], None).unwrap(), 1);
    assert!(!session.message_skip);
    let next = session.operations.active_input().expect("second timed input");
    assert_eq!(next.wait.default_value, Some(era_runtime_protocol::ProtocolValue::Integer(9)));
    let replay = input_replay_records(&session);
    assert_eq!(replay[0]["step_count"], 1);
    assert_eq!(replay[1]["action"], "timeout");
    assert_eq!(replay[1]["result"]["value"], "7");
}

#[test]
fn untimed_one_input_message_skip_keeps_the_complete_default() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "test".into(),
            features: vec![RuntimeFeature::TimedInput],
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
                relative_path: "input.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nINPUT\nONEINPUTS LONG, 0, 0\nPRINTFORML got=%RESULTS%\nWAIT\nRETURN\n"
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
    let mut started = Vec::new();
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).expect("wait");
        started.extend(drain(&mut session));
        if session.operations.active_input().is_some() {
            break;
        }
    }
    let wait = session
        .operations
        .active_input()
        .unwrap_or_else(|| {
            panic!(
                "input wait was not opened in state {:?}: {started:?}",
                session.state
            )
        })
        .wait
        .clone();
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id: wait.wait_id,
            token: wait.submission_token,
            monotonic_time_ns: 10,
            intent: InputIntent::CommitText("1".into()),
            message_skip: true,
        }),
    );
    for _ in 0..8 {
        session
            .drive(RuntimeDriveBudget::default())
            .expect("resume");
    }
    drain(&mut session);
    let snapshot = session.presentation.snapshot();
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("got=LONG"))
    );
}
