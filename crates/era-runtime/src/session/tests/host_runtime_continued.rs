#[test]
fn nested_begin_returns_current_frame_then_applies_the_deferred_flow() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "begin-test".into(),
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
                relative_path: "begin.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(
                    "@SYSTEM_TITLE\nCALL GO\nFLAG:0 = 7\nRETURN\n@GO\nFONTBOLD\nSKIPDISP 1\nBEGIN FIRST\nFLAG:0 = 99\nRETURN\n@EVENTFIRST\nPRINTL entered\nWAIT\nRETURN\n"
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
            mode: StartMode::NewGame { seed: Some(9) },
        }),
    );
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "FLAG", &[0], None).unwrap(),
        7
    );
    assert!(!session.skip_print);
    let snapshot = session.presentation.snapshot();
    let line = snapshot
        .history
        .logical_lines
        .iter()
        .find(|line| projected_line_text(line) == "entered")
        .expect("EVENTFIRST output");
    let run = line
        .runs
        .iter()
        .find(|run| matches!(run, DisplayRun::Text { .. } | DisplayRun::TextLayout { .. }))
        .expect("EVENTFIRST text style");
    assert!(matches!(
        run,
        DisplayRun::Text { style, .. } | DisplayRun::TextLayout { style, .. }
            if !style.bold
    ));
}

#[test]
#[allow(clippy::too_many_lines)]
fn builtin_title_precedes_reset_data_and_initial_character_insertion() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "title-test".into(),
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
            files: vec![
                SubmittedFile {
                    relative_path: "main.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8("@EVENTFIRST\nWAIT\nRETURN\n".into()),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "GAMEBASE.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8(
                        "バージョン,1001\nタイトル,Demo\n作者,Author\n製作年,2024\n追加情報,Info\n"
                            .into(),
                    ),
                    content_hash: None,
                },
                SubmittedFile {
                    relative_path: "CHARA0.CSV".into(),
                    category: FileCategory::Csv,
                    payload: FilePayload::Utf8("番号,0\n名前,Initial\n".into()),
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
            mode: StartMode::NewGame { seed: Some(11) },
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
        if session.operations.active_input().is_some() {
            break;
        }
    }
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "CHARANUM", &[], None).unwrap(),
        0
    );
    assert_eq!(
        session
            .vm
            .as_ref()
            .unwrap()
            .vm()
            .artifact()
            .project_data
            .new_game_seed()
            .initial_characters
            .len(),
        1
    );
    let snapshot = session.presentation.snapshot();
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.alignment == LineAlignment::Center && projected_line_text(line) == "Demo"
    }));
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(
            |run| matches!(run, DisplayRun::Button { .. } if projected_run_text(run).starts_with("[0]")),
        )
    }));
    let pending = session.operations.active_input().unwrap();
    let wait_id = pending.wait.wait_id;
    let submission_token = pending.wait.submission_token;
    submit(
        &mut session,
        3,
        RuntimeMessage::Input(FrontendInput {
            wait_id,
            token: submission_token,
            intent: InputIntent::CommitText("0".into()),
            monotonic_time_ns: 0,
            message_skip: false,
        }),
    );
    for _ in 0..8 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    assert_eq!(
        read_runtime_integer(session.vm.as_ref().unwrap(), "CHARANUM", &[], None).unwrap(),
        1
    );
}

#[test]
fn runtime_metadata_queries_use_the_active_artifact_and_fiber() {
    let mut session = RuntimeSession::new(RuntimeOptions::default());
    submit(
        &mut session,
        0,
        RuntimeMessage::ClientHello(ClientHello {
            runtime_versions: VersionRange::exact(RUNTIME_PROTOCOL_VERSION),
            client_name: "metadata-test".into(),
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
                files: vec![SubmittedFile {
                    relative_path: "metadata.erb".into(),
                    category: FileCategory::Erb,
                    payload: FilePayload::Utf8(
                        "@SYSTEM_TITLE\n#DIMS VALUES, 3, 4\n#DIMS CHOICES, 5\nVARSIZE VALUES\nPRINTFORML statement={RESULT},{RESULT:1}\nCALL SIZE_OF, CHOICES\nPRINTFORML meta={VARSIZE(\"VALUES\")},{EXISTFUNCTION(\"SYSTEM_TITLE\")},{EXISTVAR(\"VALUES\")},%GETDOINGFUNCTION()%,{RESULT},%CHOICES:2%\nPRINTFORML funcs={ENUMFUNCWITH(\"SIZE\", CHOICES)},%CHOICES:0%\nPRINTFORML vars={ENUMVARWITH(\"SAVEDATA_TEXT\", CHOICES)},%CHOICES:0%\nCALL ORACLE_REFLECTION\nPRINTFORML reflection={RESULT:12},{RESULT:13},%RESULTS:8%,%RESULTS:9%\nCALL SHORT_SCOPE\nPRINTFORML scoped={RESULT:14},{RESULT:15}\nWAIT\nRETURN\n@SIZE_OF(refChoices)\n#DIMS REF refChoices, 0\nrefChoices:2 '= \"bound\"\nRESULT = VARSIZE(\"refChoices\")\nRETURN RESULT\n@ORACLE_REFLECTION\n#DIMS NAMES, 4\nRESULT:12 = ENUMFUNCWITH(\"ORACLE_REFLECTION\", NAMES)\nRESULTS:8 = %NAMES:0%\nRESULT:13 = ENUMVARWITH(\"SAVEDATA_TEXT\", NAMES)\nRESULTS:9 = %NAMES:0%\nRETURN\n@LONG_SCOPE\n#DIM CONST PAIRS = 1, 2, 3, 4, 5, 6\nRETURN\n@SHORT_SCOPE\n#DIM CONST PAIRS = 1, 2, 3, 4\nRESULT:14 = VARSIZE(\"PAIRS\")\nFOR LOCAL, 0, VARSIZE(\"PAIRS\") / 2\nRESULT:15 += PAIRS:(LOCAL * 2)\nNEXT\nRETURN\n"
                            .into(),
                    ),
                    content_hash: None,
                }],
            }),
        );
    session.drive(RuntimeDriveBudget::default()).unwrap();
    let loaded = drain(&mut session);
    assert!(
        loaded.iter().any(|message| {
            matches!(message, RuntimeMessage::ProjectLoadReport(report) if report.success)
        }),
        "{loaded:#?}"
    );
    submit(
        &mut session,
        2,
        RuntimeMessage::Start(StartRequest {
            mode: StartMode::NewGame { seed: Some(1) },
        }),
    );
    for _ in 0..24 {
        session.drive(RuntimeDriveBudget::default()).unwrap();
    }
    let output = drain(&mut session);
    let snapshot = session.presentation.snapshot();
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| { projected_line_text(line).contains("meta=3,1,0,SYSTEM_TITLE,5,bound") }),
        "{output:#?}"
    );
    let rendered = snapshot
        .history
        .logical_lines
        .iter()
        .flat_map(|line| line.runs.iter())
        .filter_map(|run| match run {
            era_runtime_protocol::DisplayRun::Text { text, .. }
            | era_runtime_protocol::DisplayRun::TextLayout { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(
        rendered.contains("statement=3,4"),
        "{rendered}\n{output:#?}"
    );
    assert!(rendered.contains("funcs=1,SIZE_OF"), "{rendered}");
    assert!(rendered.contains("vars=1,SAVEDATA_TEXT"), "{rendered}");
    assert!(
        rendered.contains("reflection=1,1,ORACLE_REFLECTION,SAVEDATA_TEXT"),
        "{rendered}\n{output:#?}"
    );
    assert!(rendered.contains("scoped=4,4"), "{rendered}\n{output:#?}");
}

#[test]
fn reference_presentation_fixture_preserves_logical_intent() {
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

    // Keep this body identical to ORACLE_PRESENTATION in the C# fixture so
    // the oracle and Rust tests exercise the same EraBasic commands.
    let source = "@SYSTEM_TITLE\nPRINTBUTTON \"A\", 1\nPRINTBUTTONC \"B\", 2\nPRINTBUTTONLC \"C\", 3\nPRINTL\nDRAWLINE\nNOSKIP\nPRINTL VISIBLE\nENDNOSKIP\nRETURN\n";
    submit(
        &mut session,
        1,
        RuntimeMessage::ProjectManifest(ProjectManifest {
            project_revision: 1,
            files: vec![SubmittedFile {
                relative_path: "main.erb".into(),
                category: FileCategory::Erb,
                payload: FilePayload::Utf8(source.into()),
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
    for _ in 0..16 {
        session.drive(RuntimeDriveBudget::default()).expect("run");
        if session.phase() == RuntimePhase::Ready {
            break;
        }
    }
    drain(&mut session);
    let snapshot = session.presentation.snapshot();

    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs.iter().any(|run| {
            matches!(
                run,
                era_runtime_protocol::DisplayRun::Button { runs, .. }
                    if runs.iter().any(|run| matches!(
                        run,
                        era_runtime_protocol::DisplayRun::Text { text, .. }
                            | era_runtime_protocol::DisplayRun::TextLayout { text, .. }
                            if text == "A"
                    ))
            )
        })
    }));
    assert_eq!(
        snapshot
            .history
            .logical_lines
            .iter()
            .flat_map(|line| &line.runs)
            .filter(|run| matches!(run, era_runtime_protocol::DisplayRun::ColumnCell { .. }))
            .count(),
        2
    );
    assert!(snapshot.history.logical_lines.iter().any(|line| {
        line.runs
            .iter()
            .any(|run| matches!(run, era_runtime_protocol::DisplayRun::Separator { .. }))
    }));
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line) == "VISIBLE")
    );
}

fn flattened_display_text(runs: &[DisplayRun]) -> String {
    runs.iter()
        .map(|run| match run {
            DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => text.clone(),
            DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
                flattened_display_text(runs)
            }
            _ => String::new(),
        })
        .collect()
}

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
                            "../../../../../tools/runtime-tester/fixture-reference/erb/print-family.erb"
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
                preferred_columns,
            } => Some((content, alignment, preferred_columns)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 4);
    assert_eq!(*cells[0].1, era_runtime_protocol::CellAlignment::Right);
    assert_eq!(*cells[1].1, era_runtime_protocol::CellAlignment::Left);
    assert!(cells.iter().all(|cell| *cell.2 == 24));
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
    assert_eq!(session.command_intents.len(), 4);
}

#[test]
fn matching_timed_input_wins_over_queued_timer_and_starts_message_skip() {
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
    drain(&mut session);
    let snapshot = session.presentation.snapshot();
    assert!(
        snapshot
            .history
            .logical_lines
            .iter()
            .any(|line| projected_line_text(line).contains("got=9"))
    );
    let replay = input_replay_records(&session);
    assert_eq!(replay[0]["step_count"], 1);
    assert_eq!(replay[1]["action"], "text");
    assert_eq!(replay[1]["result"]["value"], "42");
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

